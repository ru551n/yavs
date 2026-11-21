-- LRM 14.7.3: the driving value of an out port is a source of its actual, so it belongs to the
-- actual's subtype; 10 does not belong to `integer range 0 to 9` (#1183). The subtypes are
-- not compatible (2008 14.3.5), but the error is the value, at run time.
entity src is
  port (o : out integer := 0);
end entity;
architecture a of src is
begin
  process
  begin
    wait for 1 ns;
    o <= 10;
    wait;
  end process;
end architecture;

entity top is end entity;
architecture a of top is
  signal s : integer range 0 to 9 := 0;
begin
  u : entity work.src port map (o => s);
  process
  begin
    wait for 2 ns;
    report "RES s=" & integer'image(s);
    wait;
  end process;
end architecture;
