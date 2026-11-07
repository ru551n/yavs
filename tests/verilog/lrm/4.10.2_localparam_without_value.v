// LRM 4.10.2: a localparam has a value
// expect: E4100 5:15 no value
`timescale 1ns/1ps
module m;
  localparam a;
endmodule
