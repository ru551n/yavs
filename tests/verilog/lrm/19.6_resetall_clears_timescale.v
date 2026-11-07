// LRM 19.6: `resetall puts the directives back to their defaults, `timescale included (recalled): the next module has none in force
// units: m
// expect: W4006 6:8 no timescale after `resetall
`timescale 1ns/1ps
`resetall
module m;
endmodule
