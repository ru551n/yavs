// LRM 19.3.2: `undef is followed by a macro name
// expect: E4009 4:1 no macro name
`timescale 1ns/1ps
`undef
module m;
endmodule
