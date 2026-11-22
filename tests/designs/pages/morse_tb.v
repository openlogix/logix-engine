// Testbench for morse_beacon: reads the LED's blinks back into text, the
// way a person who knows Morse code would, and checks that it says
// "HELLO FPGA" twice, with every dot, dash and gap the right length.
// Run Tests checks it; Simulate shows the LED and the decoded text.

`timescale 1ns / 1ps

// Reports an error, with both values, when actual_val isn't expected_val.
`define EXPECT_EQ(actual_val, expected_val) if ((actual_val) !== (expected_val)) $error("expected %0s, got %0s", (expected_val), (actual_val));

module morse_tb;

  localparam UNIT = 4;   // Clock cycles per unit: short, so the test is quick.

  reg clk = 1'b0;
  wire led_on;

  morse_beacon #(.UNIT_CYCLES(UNIT)) dut (.clk(clk), .led_on(led_on));

  always #41.667 clk = ~clk;   // 12 MHz.

  // The character a person reads from these dots and dashes, written as
  // text such as "-.-." (this table is the testbench's own, so it checks
  // the design's), or "#" for one that isn't in the table.
  function [7:0] decode(input [8*6-1:0] heard);
    case (heard)
      ".-":      decode = "A";
      "-...":    decode = "B";
      "-.-.":    decode = "C";
      "-..":     decode = "D";
      ".":       decode = "E";
      "..-.":    decode = "F";
      "--.":     decode = "G";
      "....":    decode = "H";
      "..":      decode = "I";
      ".---":    decode = "J";
      "-.-":     decode = "K";
      ".-..":    decode = "L";
      "--":      decode = "M";
      "-.":      decode = "N";
      "---":     decode = "O";
      ".--.":    decode = "P";
      "--.-":    decode = "Q";
      ".-.":     decode = "R";
      "...":     decode = "S";
      "-":       decode = "T";
      "..-":     decode = "U";
      "...-":    decode = "V";
      ".--":     decode = "W";
      "-..-":    decode = "X";
      "-.--":    decode = "Y";
      "--..":    decode = "Z";
      "-----":   decode = "0";
      ".----":   decode = "1";
      "..---":   decode = "2";
      "...--":   decode = "3";
      "....-":   decode = "4";
      ".....":   decode = "5";
      "-....":   decode = "6";
      "--...":   decode = "7";
      "---..":   decode = "8";
      "----.":   decode = "9";
      "..--..":  decode = "?";
      default:   decode = "#";
    endcase
  endfunction

  // What the reader has seen so far.
  reg [8*32-1:0] text = 0;    // The decoded text, the last character in the low byte.
  integer text_length = 0;
  reg [8*6-1:0] heard = 0;    // The dots and dashes of the character being read.
  integer cycles_in_state = 0;
  reg was_on = 1'b0;
  reg first_gap = 1'b1;       // The LED is off before the first character: not a gap.

  task add_char(input [7:0] c);
    begin
      text = {text[8*31-1:0], c};
      text_length = text_length + 1;
    end
  endtask

  // Each time the LED turns on or off, reads how long it was off or on.
  always @(posedge clk) begin
    if (led_on == was_on) begin
      cycles_in_state = cycles_in_state + 1;
    end else begin
      if (was_on) begin
        // A dot (1 unit) or a dash (3 units).
        if (cycles_in_state == 1 * UNIT) heard = {heard, "."};
        else if (cycles_in_state == 3 * UNIT) heard = {heard, "-"};
        else $error("an on time of %0d cycles: neither a dot nor a dash", cycles_in_state);
      end else if (first_gap) begin
        first_gap = 1'b0;
      end else if (cycles_in_state == 3 * UNIT || cycles_in_state == 7 * UNIT) begin
        // The end of a character, and after 7 units of a word too.
        add_char(decode(heard));
        if (cycles_in_state == 7 * UNIT) add_char(" ");
        heard = 0;
      end else if (cycles_in_state != 1 * UNIT) begin
        $error("an off time of %0d cycles: not a gap of 1, 3 or 7 units", cycles_in_state);
      end
      was_on = led_on;
      cycles_in_state = 1;
    end
  end

  initial begin
    $dumpvars(0, morse_tb);

    // Waits until the message has been read twice.
    wait (text_length == 22);
    `EXPECT_EQ(text[8*22-1:0], "HELLO FPGA HELLO FPGA ")

    $display("Read: %0s", text[8*22-1:0]);
    $display("End of simulation");
    $finish;
  end

endmodule
