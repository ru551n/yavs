`timescale 1ps / 1ps
// Parameters: string values, 256-bit values beyond 64 bits, real, localparam, overrides, defparam.
module parameters_leaf #(
    parameter [255:0] INIT = 256'hDEADBEEF_00000000_12345678_9ABCDEF0_00000000_CAFEBABE_0000FFFF_80000001,
    parameter [8*5-1:0] MODE = "FALSE",
    parameter SIM_DEVICE = "ULTRASCALE_PLUS",
    parameter real PERIOD = 2.5,
    parameter integer N = 4
) (
    input  [N-1:0] d,
    output [N-1:0] q
);
    localparam integer LOG2N = $clog2(N);
    localparam [LOG2N:0] ZERO = 0;
    localparam [1:0] STATE = (MODE == "TRUE") ? 2'b11 : 2'b00;
    localparam SPEED = (SIM_DEVICE == "ULTRASCALE_PLUS") ? 1 : 0;
    assign q = d ^ INIT[N-1:0] ^ STATE ^ SPEED;
endmodule

module parameters (
    input  [7:0] d,
    output [7:0] q1,
    output [3:0] q2
);
    parameters_leaf #(.N(8), .MODE("TRUE")) u1 (.d(d), .q(q1));
    parameters_leaf #(256'h1, "FALSE", "7SERIES", 1.5, 4) u2 (.d(d[3:0]), .q(q2));
    parameters_leaf u3 (.d(4'b0), .q());
    defparam u3.N = 4;
endmodule
