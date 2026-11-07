// LRM 19.4: a `define in a skipped branch has no effect
// units: b
// expect: none
`timescale 1ns/1ps
`ifdef NEVER
`define LATE 1
`endif
`ifdef LATE
module a;
endmodule
`else
module b;
endmodule
`endif
