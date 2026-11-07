// No timescale: the default of 1ns/1ps applies and a warning says so.
module timescale_missing (input a, output y);
    assign #1 y = a;
endmodule
