`timescale 1ps / 1ps
// Timing checks: parsed, not evaluated. Under XIL_TIMING they are the bulk of a vendor file.
module specify_timing_checks (input clk, d, ce, output reg q);
`ifdef XIL_TIMING
    reg notifier;
`endif
    always @(posedge clk) if (ce) q <= d;
    specify
        (clk => q) = 100;
`ifdef XIL_TIMING
        $setuphold (posedge clk, posedge d, 10, 5, notifier);
        $setup (d, posedge clk, 10);
        $hold (posedge clk, d, 5);
        $recovery (posedge ce, posedge clk, 2);
        $removal (posedge ce, posedge clk, 2);
        $recrem (posedge ce, posedge clk, 2, 2);
        $skew (posedge clk, negedge clk, 1);
        $period (posedge clk, 100);
        $width (posedge clk, 40);
        $nochange (posedge clk, d, 0, 0);
`endif
    endspecify
endmodule
