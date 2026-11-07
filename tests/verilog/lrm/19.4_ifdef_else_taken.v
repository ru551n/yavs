// LRM 19.4: `else compiles its text when the macro is not defined
// units: b
// expect: none
`timescale 1ns/1ps
`ifdef X
module a;
endmodule
`else
module b;
endmodule
`endif
