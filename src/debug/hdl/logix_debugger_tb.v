// Testbench for logix_debugger.v: plays the host over the UART, a few
// clocks per bit, and checks every command's reply and effect.
//
// Run with Icarus Verilog (iverilog and vvp are in the engine's toolchain):
//   iverilog -g2005 -o tb.vvp logix_debugger_tb.v logix_debugger.v && vvp tb.vvp

`timescale 1ns / 1ps

module logix_debugger_tb;

  localparam integer CPB = 8;
  localparam [31:0] ID = 32'hC0DE1234;

  reg clk = 0;
  always #5 clk = ~clk;

  reg host_tx = 1;  // to the debugger's rx
  wire host_rx;
  reg [19:0] rd_in = 20'hABCDE;
  wire [11:0] wr_out;
  wire [1:0] ovr_en;
  wire [1:0] pulse;

  // Clocks each pulse output was high, and the last pulse's value.
  integer pulse_clocks = 0;
  reg [1:0] pulse_seen = 2'b00;
  always @(posedge clk)
    if (pulse != 0) begin
      pulse_clocks = pulse_clocks + 1;
      pulse_seen = pulse;
    end

  // wr_out and ovr_en must change together, in one clock.
  reg [13:0] last_out = 14'h0;
  integer out_changes = 0;
  always @(posedge clk)
    if ({ovr_en, wr_out} !== last_out) begin
      out_changes = out_changes + 1;
      last_out = {ovr_en, wr_out};
    end

  logix_debugger #(
      .CLKS_PER_BIT(CPB),
      .READ_W(20),
      .WRITE_W(12),
      .OVR_W(2),
      .BUILD_ID(ID)
  ) dut (
      .clk(clk),
      .rx(host_tx),
      .tx(host_rx),
      .rd_in(rd_in),
      .wr_out(wr_out),
      .ovr_en(ovr_en),
      .pulse(pulse)
  );

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

  task send_byte(input [7:0] b);
    integer i;
    begin
      host_tx = 0;
      repeat (CPB) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin
        host_tx = b[i];
        repeat (CPB) @(posedge clk);
      end
      host_tx = 1;
      repeat (CPB) @(posedge clk);
    end
  endtask

  task recv_byte(output [7:0] b);
    integer i;
    begin
      @(negedge host_rx);
      repeat (CPB / 2) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin
        repeat (CPB) @(posedge clk);
        b[i] = host_rx;
      end
      repeat (CPB) @(posedge clk);
      if (host_rx !== 1) $fatal(1, "bad stop bit");
    end
  endtask

  reg [7:0] req[0:15];
  reg [7:0] rep[0:255];
  integer rep_len, rep_status;

  // Sends a frame of n payload bytes from req, with a CRC (bad if corrupt).
  task send_frame(input [7:0] command, input integer n, input corrupt);
    integer i;
    reg [7:0] c;
    begin
      send_byte(8'hA5);
      c = crc8(8'h00, command);
      send_byte(command);
      c = crc8(c, n[7:0]);
      send_byte(n[7:0]);
      for (i = 0; i < n; i = i + 1) begin
        c = crc8(c, req[i]);
        send_byte(req[i]);
      end
      send_byte(corrupt ? ~c : c);
    end
  endtask

  // Reads a reply into rep, checking its sync, command and CRC.
  task get_reply(input [7:0] command);
    integer i;
    reg [7:0] c, b;
    begin
      recv_byte(b);
      if (b !== 8'h5A) $fatal(1, "reply sync %h", b);
      recv_byte(b);
      if (b !== command) $fatal(1, "reply command %h, expected %h", b, command);
      c = crc8(8'h00, b);
      recv_byte(b);
      rep_status = b;
      c = crc8(c, b);
      recv_byte(b);
      rep_len = b;
      c = crc8(c, b);
      for (i = 0; i < rep_len; i = i + 1) begin
        recv_byte(rep[i]);
        c = crc8(c, rep[i]);
      end
      recv_byte(b);
      if (b !== c) $fatal(1, "reply CRC %h, expected %h", b, c);
    end
  endtask

  // A frame, then its reply.
  task frame(input [7:0] command, input integer n, input corrupt);
    begin
      send_frame(command, n, corrupt);
      get_reply(command);
    end
  endtask

  task expect_status(input integer s, input integer n);
    begin
      if (rep_status !== s) $fatal(1, "status %0d, expected %0d", rep_status, s);
      if (rep_len !== n) $fatal(1, "reply length %0d, expected %0d", rep_len, n);
    end
  endtask

  // A READ: the read nets, the write values and the enables, packed: bits
  // 0-19, 20-31 and 32-33 of 5 bytes.
  task expect_read(input [19:0] r, input [11:0] w, input [1:0] e);
    reg [39:0] got;
    begin
      expect_status(0, 5);
      got = {rep[4], rep[3], rep[2], rep[1], rep[0]};
      if (got !== {6'd0, e, w, r}) $fatal(1, "read %h, expected %h", got, {6'd0, e, w, r});
    end
  endtask

  initial begin
    repeat (20) @(posedge clk);

    // INFO
    frame(8'h01, 0, 0);
    expect_status(0, 5);
    if (rep[0] !== 1) $fatal(1, "protocol %0d", rep[0]);
    if ({rep[4], rep[3], rep[2], rep[1]} !== ID) $fatal(1, "build ID");

    // READ: a snapshot, taken when the frame is in.
    frame(8'h02, 0, 0);
    expect_read(20'hABCDE, 12'h000, 2'b00);
    rd_in = 20'h12345;
    frame(8'h02, 0, 0);
    expect_read(20'h12345, 12'h000, 2'b00);

    // WRITE: values and enables in one clock, read back by READ. Packed:
    // 12 bits of values, then 2 of enables.
    req[0] = 8'h21; req[1] = 8'h23;
    out_changes = 0;
    frame(8'h03, 2, 0);
    expect_status(0, 0);
    if (wr_out !== 12'h321 || ovr_en !== 2'b10) $fatal(1, "wr_out %h ovr_en %b", wr_out, ovr_en);
    if (out_changes !== 1) $fatal(1, "WRITE changed the outputs in %0d clocks", out_changes);
    frame(8'h02, 0, 0);
    expect_read(20'h12345, 12'h321, 2'b10);

    // WRITE of the wrong length, or with a bad CRC, changes nothing.
    req[0] = 8'hFF; req[1] = 8'hFF; req[2] = 8'hFF;
    frame(8'h03, 3, 0);
    expect_status(3, 0);
    frame(8'h03, 1, 0);
    expect_status(3, 0);
    frame(8'h03, 2, 1);
    expect_status(1, 0);
    if (wr_out !== 12'h321 || ovr_en !== 2'b10) $fatal(1, "refused WRITE took effect");

    // Release everything.
    req[0] = 0; req[1] = 0;
    frame(8'h03, 2, 0);
    if (wr_out !== 12'h000 || ovr_en !== 2'b00) $fatal(1, "release");

    // PULSE: high for exactly one clock, only the masked bits, and the
    // enables are untouched.
    req[0] = 8'h01;
    pulse_clocks = 0;
    frame(8'h04, 1, 0);
    expect_status(0, 0);
    if (pulse_clocks !== 1 || pulse_seen !== 2'b01) $fatal(1, "pulse %0d clocks, %b", pulse_clocks, pulse_seen);
    if (ovr_en !== 2'b00) $fatal(1, "pulse changed ovr_en %b", ovr_en);
    req[0] = 8'h03;
    pulse_clocks = 0;
    frame(8'h04, 1, 0);
    if (pulse_clocks !== 1 || pulse_seen !== 2'b11) $fatal(1, "pulse both %0d clocks, %b", pulse_clocks, pulse_seen);
    pulse_clocks = 0;
    frame(8'h04, 2, 0);
    expect_status(3, 0);
    if (pulse_clocks !== 0) $fatal(1, "a refused PULSE pulsed");

    // Bad CRC, unknown command, bad length.
    frame(8'h01, 0, 1);
    expect_status(1, 0);
    send_frame(8'h7E, 0, 0);
    get_reply(8'h00);
    expect_status(2, 0);
    frame(8'h01, 1, 0);
    expect_status(3, 0);
    frame(8'h02, 1, 0);
    expect_status(3, 0);

    // A frame cut short is dropped, and the next one works.
    send_byte(8'hA5);
    send_byte(8'h01);
    repeat (CPB * 600) @(posedge clk);
    frame(8'h01, 0, 0);
    expect_status(0, 5);

    // INFO, WRITE and READ sent back to back, without waiting: the
    // receive buffer holds what arrives while a reply goes out.
    rd_in = 20'h5A5A5;
    req[0] = 8'hBC; req[1] = 8'h1A;
    fork
      begin
        send_frame(8'h01, 0, 0);
        send_frame(8'h03, 2, 0);
        send_frame(8'h02, 0, 0);
      end
      begin
        get_reply(8'h01);
        expect_status(0, 5);
        get_reply(8'h03);
        expect_status(0, 0);
        get_reply(8'h02);
        expect_read(20'h5A5A5, 12'hABC, 2'b01);
      end
    join

    $display("OK: logix_debugger passes");
    $finish;
  end

  initial begin
    #50_000_000;
    $fatal(1, "timeout");
  end

endmodule
