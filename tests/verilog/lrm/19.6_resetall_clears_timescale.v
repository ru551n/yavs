// LRM 19.6: `resetall puts every directive back to its default; 19.8 names the timescale among them: the next module has none in force
// units: m
// expect: W4006 6:8 no timescale after `resetall
`timescale 1ns/1ps
`resetall
module m;
endmodule
