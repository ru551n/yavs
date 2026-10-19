-- LRM 8.7, 16.2.5: an external constant whose value is computed when the design is elaborated
-- ('path_name), read from another instance.
entity dut is end;
architecture a of dut is
  constant c1 : integer := 1;
  constant c2 : string := c1'path_name;
begin
end;
entity top is end;
architecture t of top is
begin
  dut : entity work.dut;
  process
  begin
    report "RES " & <<constant .top.dut.c2 : string>>;
    wait;
  end process;
end;
