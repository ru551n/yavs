// LRM 12.3.4: ports are declared in the header with a direction and an optional type and range
// units: m
// expect: none
`timescale 1ns/1ps
module m (input a, input [3:0] b, output reg [3:0] c, output wire d, inout e, input wire signed [1:0] f);
endmodule
