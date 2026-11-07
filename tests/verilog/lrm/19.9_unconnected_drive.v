// LRM 19.9: `unconnected_drive pull0 and pull1 are legal and are reset by `nounconnected_drive
// units: m
// expect: none
`timescale 1ns/1ps
`unconnected_drive pull1
module m;
endmodule
`nounconnected_drive
