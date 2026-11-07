`timescale 1ns / 1ps
module unbalanced_ifdef (input a, output y);
`ifdef SOME_FLAG
    assign y = a;
endmodule
