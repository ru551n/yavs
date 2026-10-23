-- yavs-test: std=2008
-- expect: E1215 10:33 the range is not inside type mark `POSITIVE`
-- expect: E1215 12:31 the range is not inside type mark `pos3`
-- LRM 5.2.1, 6.3 (#465)
entity e is end;
architecture a of e is
  subtype pos3 is positive range 1 to 3;
begin
  process
    variable x : positive range 0 to 3;
    variable ok : pos3 range 2 to 3;
    variable bad : pos3 range 2 to 4;
  begin
    wait;
  end process;
end;
