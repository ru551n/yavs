// LRM 19.2: `default_nettype none is a legal setting
// units: m
// expect: none
`timescale 1ns/1ps
`default_nettype none
module m;
endmodule
`default_nettype wire
