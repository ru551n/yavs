// LRM 19.6: `resetall is illegal inside a module or UDP declaration (known gap G8: not checked)
// units: m
// gap: G8 a directive inside a module or UDP is accepted
// expect: none
`timescale 1ns/1ps
module m;
`resetall
endmodule
