-- LRM 10.5.2.1, 9.3.3.3: range choices with array values in an aggregate target.
entity top is end entity;
architecture a of top is
begin
  process
    variable v : bit_vector(5 downto 0) := "110010";
    variable hi : bit_vector(5 downto 2);
    variable lo : bit_vector(1 downto 0);
  begin
    (5 downto 2 => hi, 1 downto 0 => lo) := v;
    report "RES hi=" & to_string(hi) & " lo=" & to_string(lo);
    wait;
  end process;
end architecture;
