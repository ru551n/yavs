`timescale 1ns / 1ps
// SystemVerilog syntax in a .v file: always_ff cannot be a 1364 identifier here.
module sv_keyword (input clk, d, output reg q);
    always_ff @(posedge clk) q <= d;
endmodule
