`timescale 1ns / 1ps
// Near miss for the W4007 envelope notice: begin/end mark plaintext that a tool would encrypt; they are not an envelope.
`pragma protect begin
module pragma_plain (input a, output y);
    assign y = a;
endmodule
`pragma protect end
