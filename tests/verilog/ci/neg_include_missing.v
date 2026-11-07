// Arguments for this file are in fixtures.expected.
`timescale 1ns / 1ps
`include "no_such_header.vh"
module include_missing (input a, output y);
    assign y = a;
endmodule
