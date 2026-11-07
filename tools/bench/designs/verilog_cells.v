// The Verilog half of `bench_verilog_cells`: one cell of a netlist, an LFSR, a counter, an
// adder and a little logic between them, all in the Verilog domain.
`timescale 1ns/1ps
module vcell #(parameter [15:0] SEED = 16'h1) (
  input clk,
  input [15:0] din,
  output [15:0] q,
  output [15:0] sum
);
  reg [15:0] lfsr = SEED;
  reg [15:0] cnt = 0;
  reg [15:0] acc = 0;
  wire [15:0] a0, a1, t;
  assign a0 = lfsr ^ din;
  assign a1 = {lfsr[7:0], lfsr[15:8]} + cnt;
  assign t = ((a0 + a1) ^ (a0 & a1)) | (cnt >> 3);
  always @(posedge clk) begin
    lfsr <= {lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]};
    cnt <= cnt + 1;
    if (t[0])
      acc <= acc + t[15:4];
    else
      acc <= acc - {12'd0, t[3:0]};
  end
  assign q = lfsr;
  assign sum = acc;
endmodule
