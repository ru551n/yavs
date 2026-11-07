// LRM 19.2: `default_nettype stays in force across modules until changed
// expect: E4201 8:14 setting still none in module b
`timescale 1ns/1ps
`default_nettype none
module a (input x);
endmodule
module b (output o, input x);
  assign o = missing;
endmodule
`default_nettype wire
