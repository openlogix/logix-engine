// A Morse code beacon: blinks MESSAGE on one LED, over and over.
//
// Timing, in units (one unit is UNIT_MS milliseconds): a dot is on for 1
// unit and a dash for 3; the LED is off for 1 unit between the dots and
// dashes of a character, 3 between characters, and 7 between words and
// after the message.
//
// To use it in another design, give it the board's clock frequency and
// connect led_on to an LED (inverted for an LED that lights when 0).

module morse_beacon #(
  parameter CLOCK_HZ = 12_000_000,                    // The clock's frequency.
  parameter UNIT_MS = 120,                            // 120 ms: about 10 words a minute.
  parameter UNIT_CYCLES = CLOCK_HZ / 1000 * UNIT_MS   // Clock cycles per unit.
) (
  input clk,
  output reg led_on = 1'b0   // 1 while a dot or dash is sent.
);

  // The message: capital letters, digits and single spaces between words.
  localparam MESSAGE_LENGTH = 10;
  localparam [8*MESSAGE_LENGTH-1:0] MESSAGE = "HELLO FPGA";
  localparam INDEX_BITS = $clog2(MESSAGE_LENGTH);   // Bits for a character's index.
  localparam [INDEX_BITS-1:0] LAST_INDEX = MESSAGE_LENGTH - 1;
  localparam [INDEX_BITS-1:0] PAST_SPACE = 2;   // From a word's last character to the next word.

  // The message's character at index i (0 is the first).
  function [7:0] char_at(input [INDEX_BITS-1:0] i);
    char_at = MESSAGE[8*(MESSAGE_LENGTH-1-i) +: 8];
  endfunction

  // The Morse code of a character: how many dots and dashes it has, then
  // the dots and dashes themselves, the first one in the top bit (0 is a
  // dot, 1 a dash). A character without a code is sent as "?".
  function [8:0] morse(input [7:0] char);
    case (char)
      "A": morse = {3'd2, 6'b01_0000};   // .-
      "B": morse = {3'd4, 6'b1000_00};   // -...
      "C": morse = {3'd4, 6'b1010_00};   // -.-.
      "D": morse = {3'd3, 6'b100_000};   // -..
      "E": morse = {3'd1, 6'b0_00000};   // .
      "F": morse = {3'd4, 6'b0010_00};   // ..-.
      "G": morse = {3'd3, 6'b110_000};   // --.
      "H": morse = {3'd4, 6'b0000_00};   // ....
      "I": morse = {3'd2, 6'b00_0000};   // ..
      "J": morse = {3'd4, 6'b0111_00};   // .---
      "K": morse = {3'd3, 6'b101_000};   // -.-
      "L": morse = {3'd4, 6'b0100_00};   // .-..
      "M": morse = {3'd2, 6'b11_0000};   // --
      "N": morse = {3'd2, 6'b10_0000};   // -.
      "O": morse = {3'd3, 6'b111_000};   // ---
      "P": morse = {3'd4, 6'b0110_00};   // .--.
      "Q": morse = {3'd4, 6'b1101_00};   // --.-
      "R": morse = {3'd3, 6'b010_000};   // .-.
      "S": morse = {3'd3, 6'b000_000};   // ...
      "T": morse = {3'd1, 6'b1_00000};   // -
      "U": morse = {3'd3, 6'b001_000};   // ..-
      "V": morse = {3'd4, 6'b0001_00};   // ...-
      "W": morse = {3'd3, 6'b011_000};   // .--
      "X": morse = {3'd4, 6'b1001_00};   // -..-
      "Y": morse = {3'd4, 6'b1011_00};   // -.--
      "Z": morse = {3'd4, 6'b1100_00};   // --..
      "0": morse = {3'd5, 6'b11111_0};   // -----
      "1": morse = {3'd5, 6'b01111_0};   // .----
      "2": morse = {3'd5, 6'b00111_0};   // ..---
      "3": morse = {3'd5, 6'b00011_0};   // ...--
      "4": morse = {3'd5, 6'b00001_0};   // ....-
      "5": morse = {3'd5, 6'b00000_0};   // .....
      "6": morse = {3'd5, 6'b10000_0};   // -....
      "7": morse = {3'd5, 6'b11000_0};   // --...
      "8": morse = {3'd5, 6'b11100_0};   // ---..
      "9": morse = {3'd5, 6'b11110_0};   // ----.
      default: morse = {3'd6, 6'b001100}; // ..--.. ("?")
    endcase
  endfunction

  // A pulse once per unit: the beacon's heartbeat.
  // (32 bits are plenty; the build keeps only the bits it needs.)
  reg [31:0] cycle_count = 0;
  wire unit_tick = (cycle_count == UNIT_CYCLES - 1);
  always @(posedge clk) cycle_count <= unit_tick ? 0 : cycle_count + 1'b1;

  // Lengths of the dots, dashes and gaps, in units.
  localparam DOT = 1, DASH = 3, ELEMENT_GAP = 1, LETTER_GAP = 3, WORD_GAP = 7;

  reg started = 1'b0;                                    // The first character is loaded.
  reg [INDEX_BITS-1:0] char_index = 0;                   // The character being sent.
  reg [2:0] elements_left = 0;                           // Its dots and dashes still to send.
  reg [5:0] pattern = 0;                                 // Those dots and dashes, the next in the top bit.
  reg [2:0] units_left = 0;                              // Units left in the current on or off time.

  // Gets the character at index i ready to send.
  task load(input [INDEX_BITS-1:0] i);
    begin
      char_index <= i;
      {elements_left, pattern} <= morse(char_at(i));
    end
  endtask

  always @(posedge clk) begin
    if (!started) begin
      load(0);
      started <= 1'b1;
    end else if (unit_tick) begin
      if (units_left != 0) begin
        // The current on or off time goes on.
        units_left <= units_left - 1'b1;
      end else if (!led_on) begin
        // A gap has ended: send the next dot or dash.
        led_on <= 1'b1;
        units_left <= (pattern[5] ? DASH : DOT) - 1;
        pattern <= pattern << 1;
        elements_left <= elements_left - 1'b1;
      end else begin
        // A dot or dash has ended: turn the LED off for the gap after it.
        led_on <= 1'b0;
        if (elements_left != 0) begin
          // More dots and dashes in this character.
          units_left <= ELEMENT_GAP - 1;
        end else if (char_index == LAST_INDEX) begin
          // The message is done: a word's gap, then start over.
          units_left <= WORD_GAP - 1;
          load(0);
        end else if (char_at(char_index + 1'b1) == " ") begin
          // The word is done: a word's gap, then the next word.
          units_left <= WORD_GAP - 1;
          load(char_index + PAST_SPACE);
        end else begin
          // The character is done: a letter's gap, then the next one.
          units_left <= LETTER_GAP - 1;
          load(char_index + 1'b1);
        end
      end
    end
  end

endmodule
