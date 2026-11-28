// Logix debugger (docs/debugger.md, Debugger hardware).
//
// A register-map debugger behind a UART: the host sends frames, the
// debugger answers them. Plain Verilog-2005 with no FPGA primitives, so it
// builds for every family. One clock for everything (version 1).
//
// Host frame:  A5, command, length, payload (length bytes), CRC-8
// Reply frame: 5A, command, status, length, payload (length bytes), CRC-8
// CRC-8: polynomial 07, initial 00, over every byte after the sync byte.
//
// Commands (protocol 1). Spaces are bits packed low first, little-endian,
// padded with zeros to whole bytes:
//   01 INFO, no payload: replies the protocol and the build ID (4 bytes,
//      low first).
//   02 READ, no payload: takes a snapshot in one clock and replies with it:
//      the read nets, then the write values, then the override enables.
//   03 WRITE: the write values, then the enables. All take effect in the
//      same clock, once the frame's CRC checks out.
//   04 PULSE: a mask of the enables' width. Each set bit's pulse output is
//      high for exactly one clock, all of them in the same clock.
// Status: 0 OK, 1 bad CRC, 2 unknown command, 3 bad length. A reply to an
// unknown command has command 00.
//
// Built for size and speed: replies leave through a byte shift register,
// payloads come in through the same one, one CRC serves both ways, and
// actions run from registered strobes. No path here is more than a few LUTs
// long, so the debugger doesn't set the design's maximum clock.

module logix_debugger #(
    parameter integer CLKS_PER_BIT = 104,  // clock Hz / baud
    parameter integer READ_W = 1,          // bits of read nets, packed
    parameter integer WRITE_W = 1,         // bits of write nets, packed
    parameter integer OVR_W = 1,           // one enable per write net
    parameter [31:0] BUILD_ID = 32'h0
) (
    input  wire               clk,
    input  wire               rx,
    output wire               tx,
    input  wire [ READ_W-1:0] rd_in,
    output wire [WRITE_W-1:0] wr_out,
    output wire [  OVR_W-1:0] ovr_en,
    output wire [  OVR_W-1:0] pulse   // one clock high, per write net
);

  localparam [7:0] PROTOCOL = 8'd1;
  localparam integer SB = (READ_W + WRITE_W + OVR_W + 7) / 8;  // READ's reply
  localparam integer KB = (WRITE_W + OVR_W + 7) / 8;  // WRITE's payload
  localparam integer EB = (OVR_W + 7) / 8;  // PULSE's payload
  localparam integer IB = 5;  // INFO's reply
  localparam integer XB = SB > IB ? SB : IB;  // the shift register
  // A frame that stops arriving is dropped after 256 to 512 bit times
  // (25 to 50 bytes): when the idle counter's top bit sets.
  localparam integer TW = $clog2(CLKS_PER_BIT * 256) + 1;

  // --- UART ---------------------------------------------------------------

  wire [7:0] uart_data;
  wire       uart_valid;
  reg  [7:0] nb = 8'h00;  // the next reply byte
  reg        tx_start = 1'b0;
  wire       tx_busy;

  logix_debugger_uart_rx #(.CLKS_PER_BIT(CLKS_PER_BIT)) u_rx (
      .clk  (clk),
      .rx   (rx),
      .data (uart_data),
      .valid(uart_valid)
  );

  logix_debugger_uart_tx #(.CLKS_PER_BIT(CLKS_PER_BIT)) u_tx (
      .clk  (clk),
      .data (nb),
      .start(tx_start),
      .tx   (tx),
      .busy (tx_busy)
  );

  // --- Registers ----------------------------------------------------------

  reg [WRITE_W-1:0] wr_reg = 0;
  reg [  OVR_W-1:0] en_reg = 0;
  reg [  OVR_W-1:0] pls_reg = 0;
  assign wr_out = wr_reg;
  assign ovr_en = en_reg;
  assign pulse  = pls_reg;

  // The shift register: replies leave from its low byte, payloads come in
  // at its top. A READ loads the snapshot, an INFO the protocol and the
  // build ID.
  reg  [XB*8-1:0] snap = 0;
  wire [XB*8-1:0] snap_in = {en_reg, wr_reg, rd_in};
  wire [XB*8-1:0] info_in = {BUILD_ID, PROTOCOL};
  // Where a payload's bits are once it is in: WRITE's values and enables,
  // PULSE's mask.
  wire [WRITE_W-1:0] st_wr = snap[(XB-KB)*8+:WRITE_W];
  wire [OVR_W-1:0] st_en = snap[(XB-KB)*8+WRITE_W+:OVR_W];
  wire [OVR_W-1:0] st_pl = snap[(XB-EB)*8+:OVR_W];

  // The actions, a clock after the frame checks out.
  reg do_info = 1'b0, do_read = 1'b0, do_write = 1'b0, do_pulse = 1'b0;
  reg do_shift = 1'b0;  // the next payload byte, after one is sent

  always @(posedge clk) begin
    if (do_write) begin
      wr_reg <= st_wr;
      en_reg <= st_en;
    end
    pls_reg <= do_pulse ? st_pl : {OVR_W{1'b0}};
  end

  function [7:0] crc8;
    input [7:0] c;
    input [7:0] d;
    integer i;
    reg [7:0] x;
    begin
      x = c ^ d;
      for (i = 0; i < 8; i = i + 1) x = x[7] ? ({x[6:0], 1'b0} ^ 8'h07) : {x[6:0], 1'b0};
      crc8 = x;
    end
  endfunction

  // --- Receive buffer -----------------------------------------------------
  // The command engine takes input only while it waits for a frame, so bytes
  // that arrive while it replies wait here. The host can then send several
  // frames at once and pay the link's round trip once. The host keeps at
  // most 48 bytes in flight, so the buffer never fills: there is no full
  // check, which would sit on the UART's write path. Each byte is stored with
  // the flags the engine would otherwise compare it for (the RAM is 16 bits
  // wide): 0 to 4, sync, and the WRITE and PULSE payload lengths.

  // The engine's states, one-hot.
  reg s_sync = 1'b1, s_cmd = 1'b0, s_len = 1'b0, s_pay = 1'b0, s_crc = 1'b0;
  reg s_chk = 1'b0, s_exec = 1'b0, s_reply = 1'b0;
  wire receiving = !(s_chk || s_exec || s_reply);

  localparam integer FIFO_DEPTH = 64;
  // A read of the slot being written may return anything: avail is low
  // then, so the byte isn't taken. Saying so saves yosys emulating it.
  (* no_rw_check *)
  reg [15:0] fifo[0:FIFO_DEPTH-1];
  reg [5:0] wp = 6'd0;
  reg [5:0] rp = 6'd0;
  reg [15:0] rx_word;  // the engine's byte while rx_valid (no initial
                      // value, so it can be the block RAM's own)
  wire [7:0] rx_data = rx_word[7:0];
  wire rx_zero = rx_word[8], rx_one = rx_word[9], rx_two = rx_word[10];
  wire rx_three = rx_word[11], rx_four = rx_word[12], rx_sync = rx_word[13];
  wire rx_kb = rx_word[14], rx_eb = rx_word[15];
  reg rx_valid = 1'b0;  // high for one clock per byte
  // Bytes are waiting, a clock late. A byte is taken at most every other
  // clock, so by the next chance to take one this has seen rp move.
  reg avail = 1'b0;

  always @(posedge clk) begin
    if (uart_valid) begin
      fifo[wp] <= {
        uart_data == EB[7:0],
        uart_data == KB[7:0],
        uart_data == 8'hA5,
        uart_data == 8'd4,
        uart_data == 8'd3,
        uart_data == 8'd2,
        uart_data == 8'd1,
        uart_data == 8'd0,
        uart_data
      };
      wp <= wp + 1'b1;
    end
    rx_word  <= fifo[rp];
    avail    <= (wp != rp);
    rx_valid <= 1'b0;
    if (receiving && avail && !rx_valid) begin
      rp       <= rp + 1'b1;
      rx_valid <= 1'b1;
    end
  end

  // --- Command engine -----------------------------------------------------

  // The command and length, decoded as they arrive.
  reg c_info = 1'b0, c_read = 1'b0, c_write = 1'b0, c_pulse = 1'b0;
  reg l_zero = 1'b0, l_kb = 1'b0, l_eb = 1'b0;
  wire known = c_info || c_read || c_write || c_pulse;
  wire len_ok = (c_info || c_read) ? l_zero : c_write ? l_kb : l_eb;
  // Ready long before the CRC byte (bytes come at most every other clock):
  // the status if the CRC checks out, and whether the frame is good then.
  reg [1:0] pre_status = 2'd0;
  reg cmd_ok = 1'b0;
  // One CRC for both ways, over the bytes received or the reply bytes sent,
  // in two clocks a byte: crc_x takes the byte in, then crc the polynomial.
  // Taking the frame's own CRC byte in leaves crc_x zero when it is right.
  reg [7:0] crc = 8'h00;
  reg [7:0] crc_x = 8'h00;
  reg crc_step = 1'b0;
  wire [7:0] crc_in = crc ^ (s_reply ? nb : rx_data);
  wire crc_take;
  reg crc_ok = 1'b0;
  reg [7:0] rleft = 8'h00;  // payload bytes still to come
  reg rlast = 1'b0;  // rleft is 1
  reg [7:0] tleft = 8'h00;  // payload bytes still to send
  reg tlast = 1'b0;  // tleft is 1
  reg [TW-1:0] idle = 0;
  reg timeout = 1'b0;

  // The reply's phases, one-hot.
  reg p_sync = 1'b1, p_cmd = 1'b0, p_status = 1'b0, p_len = 1'b0, p_data = 1'b0, p_crc = 1'b0;
  reg [1:0] status = 2'd0;
  reg has_data = 1'b0;  // the reply has a payload
  // Time for the next reply byte: the transmitter is free. Registered, and
  // low until busy has risen after the last start.
  reg go = 1'b0;
  assign crc_take = (rx_valid && !s_sync) || (go && !p_sync);

  // A byte in while receiving a payload; a byte out after one is sent.
  always @(posedge clk) begin
    if (do_read) snap <= snap_in;
    else if (do_info) snap <= info_in;
    else if (do_shift || (s_pay && rx_valid)) snap <= {rx_data, snap[XB*8-1:8]};
  end

  always @(posedge clk) begin
    nb <= ({8{p_sync}} & 8'h5A)
        | ({8{p_cmd}} & {5'd0, c_pulse, c_read || c_write, c_info || c_write})
        | ({8{p_status}} & {6'd0, status})
        | ({8{p_len}} & tleft)
        | ({8{p_data}} & snap[7:0])
        | ({8{p_crc}} & crc);
  end

  always @(posedge clk) begin
    tx_start <= 1'b0;
    do_info  <= 1'b0;
    do_read  <= 1'b0;
    do_write <= 1'b0;
    do_pulse <= 1'b0;
    do_shift <= 1'b0;
    go <= s_reply && !tx_busy && !go && !tx_start;
    // Every byte after a sync byte goes into the CRC, both ways.
    crc_step <= crc_take;
    if (crc_take) crc_x <= crc_in;
    // Each frame and each reply starts from 0.
    if (s_sync || s_exec) crc <= 8'h00;
    else if (crc_step) crc <= crc8(crc_x, 8'd0);

    // Frames that stop arriving mid-way are dropped (below).
    if (s_sync || rx_valid) idle <= 0;
    else if (receiving) idle <= idle + 1'b1;
    timeout <= receiving && idle[TW-1] && !timeout;
    pre_status <= !known ? 2'd2 : !len_ok ? 2'd3 : 2'd0;
    cmd_ok <= known && len_ok;

    if (rx_valid) begin
      if (s_sync && rx_sync) begin
        s_sync <= 1'b0;
        s_cmd  <= 1'b1;
      end
      if (s_cmd) begin
        c_info  <= rx_one;
        c_read  <= rx_two;
        c_write <= rx_three;
        c_pulse <= rx_four;
        s_cmd   <= 1'b0;
        s_len   <= 1'b1;
      end
      if (s_len) begin
        l_zero <= rx_zero;
        l_kb   <= rx_kb;
        l_eb   <= rx_eb;
        rleft  <= rx_data;
        rlast  <= rx_one;
        s_len  <= 1'b0;
        s_crc  <= rx_zero;
        s_pay  <= !rx_zero;
      end
      if (s_pay) begin
        rleft <= rleft - 1'b1;
        rlast <= (rleft == 8'd2);
        if (rlast) begin
          s_pay <= 1'b0;
          s_crc <= 1'b1;
        end
      end
      if (s_crc) begin
        s_crc <= 1'b0;
        s_chk <= 1'b1;
      end
    end

    if (s_chk) begin
      crc_ok <= (crc_x == 8'd0);
      s_chk  <= 1'b0;
      s_exec <= 1'b1;
    end

    if (s_exec) begin
      p_sync   <= 1'b1;
      s_exec   <= 1'b0;
      s_reply  <= 1'b1;
      status   <= crc_ok ? pre_status : 2'd1;
      do_info  <= crc_ok && cmd_ok && c_info;
      do_read  <= crc_ok && cmd_ok && c_read;
      do_write <= crc_ok && cmd_ok && c_write;
      do_pulse <= crc_ok && cmd_ok && c_pulse;
      has_data <= crc_ok && cmd_ok && (c_info || c_read);
      tleft    <= !(crc_ok && cmd_ok) ? 8'd0 : c_info ? IB[7:0] : c_read ? SB[7:0] : 8'd0;
      tlast    <= crc_ok && cmd_ok && ((c_info && IB == 1) || (c_read && SB == 1));
    end

    if (s_reply && go) begin
      tx_start <= 1'b1;
      if (p_sync) begin
        p_sync <= 1'b0;
        p_cmd  <= 1'b1;
      end
      if (p_cmd) begin
        p_cmd    <= 1'b0;
        p_status <= 1'b1;
      end
      if (p_status) begin
        p_status <= 1'b0;
        p_len    <= 1'b1;
      end
      if (p_len) begin
        p_len  <= 1'b0;
        p_data <= has_data;
        p_crc  <= !has_data;
      end
      if (p_data) begin
        do_shift <= 1'b1;
        tleft    <= tleft - 1'b1;
        tlast    <= (tleft == 8'd2);
        if (tlast) begin
          p_data <= 1'b0;
          p_crc  <= 1'b1;
        end
      end
      if (p_crc) begin
        p_crc   <= 1'b0;
        s_reply <= 1'b0;
        s_sync  <= 1'b1;
      end
    end

    if (timeout) begin
      s_sync <= 1'b1;
      s_cmd  <= 1'b0;
      s_len  <= 1'b0;
      s_pay  <= 1'b0;
      s_crc  <= 1'b0;
    end
  end

endmodule

// 8N1 receiver: samples each bit at its middle. data holds the byte from
// valid until the next byte's first data bit. The bit timer counts down
// past zero, and its sign bit is the tick, so no compare sits on its path.
module logix_debugger_uart_rx #(
    parameter integer CLKS_PER_BIT = 104
) (
    input  wire       clk,
    input  wire       rx,
    output wire [7:0] data,
    output reg        valid = 1'b0
);
  localparam integer CW = $clog2(CLKS_PER_BIT);
  reg r1 = 1'b1, r2 = 1'b1;
  reg busy = 1'b0;
  reg [CW:0] cnt = 0;
  wire tick = cnt[CW];
  reg [3:0] n = 4'd0;
  reg [7:0] sh = 8'h00;
  assign data = sh;

  always @(posedge clk) begin
    r1 <= rx;
    r2 <= r1;
    valid <= 1'b0;
    if (!busy) begin
      if (!r2) begin
        busy <= 1'b1;
        cnt  <= CLKS_PER_BIT / 2 - 1;  // to the middle of the start bit
        n    <= 4'd0;
      end
    end else if (!tick) cnt <= cnt - 1'b1;
    else begin
      cnt <= CLKS_PER_BIT - 2;
      n   <= n + 1'b1;
      if (n == 0) begin
        if (r2) busy <= 1'b0;  // a glitch, not a start bit
      end else if (n <= 8) sh <= {r2, sh[7:1]};
      else begin
        busy  <= 1'b0;
        valid <= r2;  // a good stop bit
      end
    end
  end
endmodule

// 8N1 transmitter. busy is high from the clock after start until the stop
// bit is done. The bit timer works as the receiver's.
module logix_debugger_uart_tx #(
    parameter integer CLKS_PER_BIT = 104
) (
    input  wire       clk,
    input  wire [7:0] data,
    input  wire       start,
    output reg        tx   = 1'b1,
    output reg        busy = 1'b0
);
  localparam integer CW = $clog2(CLKS_PER_BIT);
  reg [CW:0] cnt = {(CW + 1) {1'b1}};
  wire tick = cnt[CW];
  reg [3:0] n = 4'd0;
  reg [9:0] sh = 10'h3FF;

  always @(posedge clk) begin
    busy <= start || (n != 0) || !tick;
    if (!tick) cnt <= cnt - 1'b1;
    else if (n != 0) begin
      tx  <= sh[0];
      sh  <= {1'b1, sh[9:1]};
      n   <= n - 1'b1;
      cnt <= CLKS_PER_BIT - 2;
    end else if (start) begin
      sh <= {1'b1, data, 1'b0};
      n  <= 4'd10;
    end
  end
endmodule
