// LRM 28.4.4: an envelope that is never closed runs to the end of the file
// units: m
// expect: W4007 6:1 encrypted module body
`timescale 1ns/1ps
module m (input a, output b);
`pragma protect begin_protected
AAAA
