// LRM 12.1: the elaboration tasks $error, $warning, $info and $fatal are accepted as module items (extension V4)
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  $info("x");
  $warning;
  $error("e");
  $fatal(1, "f");
endmodule
