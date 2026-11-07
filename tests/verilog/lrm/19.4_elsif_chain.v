// LRM 19.4: `elsif tests another macro when the earlier tests failed; the first true branch is compiled
// units: b
// expect: none
`timescale 1ns/1ps
`define Y
`ifdef X
module a;
endmodule
`elsif Y
module b;
endmodule
`elsif Z
module c;
endmodule
`else
module d;
endmodule
`endif
