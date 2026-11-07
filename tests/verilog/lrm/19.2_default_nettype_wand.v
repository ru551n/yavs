// LRM 19.2: `default_nettype wand is a legal setting
// units: m
// expect: none
`timescale 1ns/1ps
`default_nettype wand
module m;
endmodule
`default_nettype wire
