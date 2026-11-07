`timescale 1ps / 1ps
// The flop idiom of every unisim: procedural assign and deassign driven by a global set/reset.
module procedural_assign #(parameter INIT = 1'b0) (
    input clk, ce, d, gsr, rst,
    output reg q
);
    reg [1:0] mode;
    initial begin
        q = INIT;
        mode = 2'b00;
    end
    always @(gsr or rst) begin
        if (gsr || rst) begin
            assign q = INIT;
        end else begin
            deassign q;
        end
    end
    always @(posedge clk) begin
        if (ce) q <= #100 d;
    end
    always @* begin
        mode = {rst, gsr};
    end
    always @(*) begin
        casex (mode)
            2'b1x: mode = mode;
            2'b01: mode = mode;
            default: mode = mode;
        endcase
    end
endmodule
