`timescale 1ns / 1ps
// trireg, wand and wor parse and analyse; they are refused when a design reaches them.
module unsupported_nettypes (input a, b, output y);
    trireg (medium) cap = a;
    trireg (small) wire_cap;
    wand w_and;
    wor  w_or;
    assign w_and = a;
    assign w_and = b;
    assign y = w_and | w_or;
endmodule
