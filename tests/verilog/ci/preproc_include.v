// An include found through the -I list; the guard in the header makes a second include harmless.
`timescale 1ns / 1ps
`include "defs.vh"
`include "defs.vh"
module preproc_include (
    input  [`DATA_W-1:0] d,
    output [`DATA_W-1:0] q
);
    assign q = `INC(d);
endmodule
