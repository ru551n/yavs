// LRM 17.2: file input and output tasks and functions
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer fd, c;
  reg [7:0] mem [0:3];
  reg [7:0] r;
  initial begin
    fd = $fopen("f.txt");
    $fdisplay(fd, "x");
    $fwrite(fd, "y");
    c = $fgetc(fd);
    c = $fscanf(fd, "%d", r);
    $fflush(fd);
    $fclose(fd);
    $readmemh("m.hex", mem);
    $readmemb("m.bin", mem, 0, 3);
    $sformat(r, "%d", c);
  end
endmodule
