// LRM 19.2: `default_nettype tri1 is a legal setting
// units: m
// expect: none
`timescale 1ns/1ps
`default_nettype tri1
module m;
endmodule
`default_nettype wire
