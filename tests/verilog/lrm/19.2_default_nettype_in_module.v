// LRM 19.2: `default_nettype is used outside module definitions (known gap G8: not checked)
// units: m
// gap: G8 a directive inside a module or UDP is accepted
// expect: none
`timescale 1ns/1ps
module m;
`default_nettype none
endmodule
