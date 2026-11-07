`timescale 1ns / 1ps
module preproc_cmdline_defs (
    input  [`DEPTH-1:0] a,
    output [`DEPTH-1:0] y
);
`ifdef USE_FAST
    assign y = a;
`else
    assign y = ~a;
`endif
endmodule
