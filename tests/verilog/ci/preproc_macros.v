// Object-like and function-like macros, nesting, undef, conditional compilation.
`timescale 1ns / 1ps
`default_nettype none
`celldefine
`define WIDTH 8
`define MAX(a, b) ((a) > (b) ? (a) : (b))
`define SUM3(a, b, c) ((a) + `MAX(b, c) + (c))
`define TWICE(x) x x
`define STR "text with `WIDTH not expanded"
module preproc_macros (
    input wire [`WIDTH-1:0] a,
    input wire [`WIDTH-1:0] b,
    output wire [`WIDTH-1:0] y
);
`ifdef FAST
    assign y = a;
`elsif WIDE
    assign y = b;
`else
`ifndef SLOW
    assign y = `SUM3(a, b, 1);
`else
    assign y = a ^ b;
`endif
`endif
    initial $display(`STR);
endmodule
`endcelldefine
`undef WIDTH
`ifdef WIDTH
module never_seen;
endmodule
`endif
`default_nettype wire
