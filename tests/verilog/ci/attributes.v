`timescale 1ns / 1ps
// Attributes: on modules, ports, nets, regs, instances and statements; parsed and ignored.
(* keep_hierarchy = "yes", CORE_GENERATION_INFO = "x,y=1" *)
module attributes (
    (* mark_debug = "true" *) input clk,
    (* dont_touch *) input d,
    (* IOB = "TRUE" *) output reg q
);
    (* ram_style = "block", async_reg = "true" *) reg [7:0] mem [0:15];
    (* keep *) wire w = d;
    (* synthesis, full_case, parallel_case *)
    always @(posedge clk) begin
        (* fsm_encoding = "one_hot" *)
        case (d)
            1'b0: q <= 1'b0;
            default: q <= w;
        endcase
    end
    (* LOC = "SLICE_X0Y0" *) attributes_leaf u (.a(d));
endmodule

module attributes_leaf (input a);
endmodule
