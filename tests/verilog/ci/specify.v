`timescale 1ps / 1ps
// Specify blocks: parallel and full paths, edge-sensitive paths, conditions, specparams, pulse control.
module specify_block (
    input clk, d, en, ce,
    input [1:0] sel,
    output reg q,
    output y,
    output [1:0] v
);
    assign y = d & en;
    assign v = sel;
    always @(posedge clk) if (ce) q <= d;
    specify
        specparam tpd = 100, tsu = 10:20:30;
        (d => y) = 100;
        (en *> y) = (tpd, tpd + 1);
        if (ce) (posedge clk => (q +: d)) = (100, 120);
        ifnone (clk => q) = 100;
        (sel[0] => v[0]) = 50;
        (sel *> v) = 50;
        showcancelled y;
        PATHPULSE$d$y = (30, 40);
    endspecify
endmodule
