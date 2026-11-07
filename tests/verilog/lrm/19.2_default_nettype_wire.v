// LRM 19.2: `default_nettype wire is a legal setting
// units: m
// expect: none
`timescale 1ns/1ps
`default_nettype wire
module m;
endmodule
`default_nettype wire
