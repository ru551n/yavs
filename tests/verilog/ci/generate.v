`timescale 1ns / 1ps
// Generate: for, if, case, nested, named blocks, genvar.
module generate_blocks #(
    parameter N = 4,
    parameter STYLE = "BIT"
) (
    input  [N-1:0] a,
    input  [N-1:0] b,
    output [N-1:0] y,
    output [N-1:0] z
);
    genvar i;
    generate
        for (i = 0; i < N; i = i + 1) begin : g_bit
            assign y[i] = a[i] & b[i];
            if (i % 2 == 0) begin : g_even
                assign z[i] = a[i];
            end else begin : g_odd
                assign z[i] = ~a[i];
            end
        end
    endgenerate
    generate
        case (STYLE)
            "BIT": begin : g_style_bit
                wire unused_b = 1'b0;
            end
            default: begin : g_style_other
                wire unused_o = 1'b1;
            end
        endcase
        if (N > 8) begin : g_big
            initial $display("wide %m");
        end
    endgenerate
endmodule
