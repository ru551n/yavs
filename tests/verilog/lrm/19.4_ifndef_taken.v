// LRM 19.4: `ifndef compiles its text when the macro is not defined
// units: a
// expect: none
`timescale 1ns/1ps
`ifndef X
module a;
endmodule
`endif
