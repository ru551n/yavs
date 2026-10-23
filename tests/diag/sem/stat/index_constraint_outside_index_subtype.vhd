-- yavs-test: std=2008
-- expect: E1215 8:29 the range is not inside index subtype `NATURAL`
-- LRM 5.3.2.2 (#465)
entity e is end;
architecture a of e is
begin
  process
    variable y : bit_vector(1 downto -1);
  begin
    wait;
  end process;
end;
