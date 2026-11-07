// LRM 3.7.2: the words only the config syntax uses (cell, design, include, ...) are ordinary identifiers here although Annex B reserves them; deliberate (V9)
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire cell, design, incdir, include, instance, liblist, library, use;
endmodule
