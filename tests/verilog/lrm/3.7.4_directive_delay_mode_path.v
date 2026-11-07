// LRM 3.7.4 and Annex D: `delay_mode_path changes the delays a simulation uses, so it is not ignored (V17)
// expect: E4010 4:1 unsupported directive
`timescale 1ns/1ps
`delay_mode_path
module m;
endmodule
