// LRM 19.6: `resetall sets `default_nettype back to wire
// units: m
// expect: none
`default_nettype none
`resetall
`timescale 1ns/1ps
module m (output o);
  assign o = missing;
endmodule
