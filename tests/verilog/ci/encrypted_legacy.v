// The legacy `protect form: everything up to `endprotect is hidden; recorded with a notice.
`timescale 1ns / 1ps
module legacy_pre (input a, output y);
    assign y = a;
endmodule
`protect
module hidden (input a, output y); assign y = ~a; endmodule
`endprotect
