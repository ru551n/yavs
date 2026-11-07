`timescale 1ns / 1ps
// ANSI port lists: directions, nets, regs, signed, vectors, inout, header parameters.
module ansi_header #(
    parameter integer W = 4,
    parameter SIGNED_OUT = 1
) (
    input wire clk,
    input wire [W-1:0] a,
    input signed [W-1:0] s,
    output reg [W-1:0] q,
    output wire signed [W:0] sum,
    inout wire pad,
    output reg ready = 1'b0
);
    assign sum = a + s;
    assign pad = ready ? q[0] : 1'bz;
    always @(posedge clk) begin
        q <= a;
        ready <= 1'b1;
    end
endmodule

module ansi_header_empty ();
endmodule

module ansi_header_old (a, y);
    input a;
    output y;
    assign y = a;
endmodule
