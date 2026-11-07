// LRM 19.9: `unconnected_drive is specified outside module declarations (known gap G8: not checked)
// units: m
// gap: G8 a directive inside a module or UDP is accepted
// expect: none
`timescale 1ns/1ps
module m;
`unconnected_drive pull1
endmodule
