// LRM 19.8: a `timescale stays in force for every later module until another one
// units: a,b
// expect: none
`timescale 1ns/1ps
module a;
endmodule
module b;
endmodule
