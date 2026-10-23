use work.ecall_pkg.all;
entity ecall_tb is end;
architecture a of ecall_tb is
  constant c1 : integer := f(3);
  constant c2 : integer := f(5);
  constant c3 : integer := f(8);
  signal s1 : bit_vector(1 to c1);
  signal s2 : bit_vector(1 to c2);
  signal s3 : bit_vector(1 to c3);
begin
  process begin
    report "c=" & integer'image(s1'length) & " " & integer'image(s2'length) & " " & integer'image(s3'length);
    wait;
  end process;
end;
