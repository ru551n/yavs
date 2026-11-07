// LRM 19.4: text in a skipped group still follows the lexical conventions: a string ends on its line (known gap G7: skipped text is not read)
// units: m
// gap: G7 lexical errors in skipped text are not reported
// expect: none
`timescale 1ns/1ps
`ifdef NEVER
initial $display("never closed);
`endif
module m;
endmodule
