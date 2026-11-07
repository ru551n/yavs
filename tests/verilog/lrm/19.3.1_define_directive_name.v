// LRM 19.3.1: a compiler directive cannot be redefined as a macro name
// expect: E4008 3:9 ifdef as a macro
`define ifdef 1
module m;
endmodule
