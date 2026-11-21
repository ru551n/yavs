-- LRM 14.7.3: a value driven through an out or inout signal formal is the driving value of the
-- actual, so it belongs to the subtype of the actual; 10 does not belong to
-- `integer range 0 to 9` (#1181).
entity top is end entity;
architecture a of top is
  signal s : integer range 0 to 9 := 0;
  procedure drv (signal o : out integer) is begin o <= 10; end procedure;
begin
  process
  begin
    drv(s);
    wait for 1 ns;
    report "RES s=" & integer'image(s);
    wait;
  end process;
end architecture;
