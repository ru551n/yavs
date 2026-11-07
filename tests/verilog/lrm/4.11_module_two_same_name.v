// LRM 4.11: module names are unique in a design
// expect: E4200 6:8 declared twice
`timescale 1ns/1ps
module m;
endmodule
module m;
endmodule
