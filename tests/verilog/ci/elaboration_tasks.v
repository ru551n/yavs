`timescale 1ns / 1ps
// $error, $warning, $info and $fatal as module and generate-block items (an extension to 1364-2005).
module elaboration_tasks #(parameter W = 8, parameter MODE = "fast") (input [W-1:0] a, output [W-1:0] y);
    $info("elaborating with W=%0d", W);
    if (W < 2)
        $error("W must be at least 2");
    generate
        if (MODE != "fast" && MODE != "slow") begin : bad_mode
            $fatal(1, "unknown MODE %s", MODE);
        end else if (W > 64) begin : wide
            $warning("W=%0d is wide", W);
        end
        case (MODE)
            "fast": begin : fast
                $info;
            end
            default: $warning();
        endcase
    endgenerate
    assign y = a;
endmodule
