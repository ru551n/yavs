-- LRM 10.6.2.1, 9.3.3.3: named associations of an aggregate target written out of index order.
entity top is end entity;
architecture a of top is
begin
  process
    variable v : bit_vector(5 downto 0) := "110010";
    variable hi : bit_vector(5 downto 2);
    variable lo : bit_vector(1 downto 0);
  begin
    (1 downto 0 => lo, 5 downto 2 => hi) := v;
    report "RES hi=" & to_string(hi) & " lo=" & to_string(lo);
    wait;
  end process;
end architecture;
