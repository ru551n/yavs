-- LRM 9.3.3.3: a range choice of elements whose direction differs from that of a range
-- choice with an array value in the same aggregate.
entity top is end entity;
architecture a of top is
begin
  process
    variable a : bit_vector(1 downto 0) := "01";
    variable v : bit_vector(3 downto 0);
  begin
    v := (3 downto 2 => a, 0 to 1 => '0');
    report "RES " & to_string(v);
    wait;
  end process;
end architecture;
