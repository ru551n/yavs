`timescale 1ns / 1ps
// Near miss for E4001: begin/end mark plaintext that a tool would encrypt; they are not an envelope.
`pragma protect begin
module pragma_plain (input a, output y);
    assign y = a;
endmodule
`pragma protect end
