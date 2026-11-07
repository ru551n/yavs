// LRM 19.3: a name after a backtick that is neither a directive nor a defined macro is an undefined macro
// expect: E4007 5:1 unknown directive or macro
`timescale 1ns/1ps
module m;
`not_a_directive
endmodule
