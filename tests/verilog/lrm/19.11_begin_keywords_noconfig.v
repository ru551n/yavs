// LRM 19.11: under "1364-2001-noconfig" the config words are identifiers
// units: m
// expect: none
`timescale 1ns/1ps
`begin_keywords "1364-2001-noconfig"
module m;
  wire config, endconfig;
endmodule
`end_keywords
