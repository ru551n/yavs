-- LRM 9.3.3.3: array values under range choices of different directions in one aggregate.
entity top is end entity;
architecture a of top is
begin
  process
    variable a : bit_vector(1 downto 0) := "01";
    variable b : bit_vector(0 to 1) := "10";
    variable v : bit_vector(3 downto 0);
  begin
    v := (0 to 1 => a, 3 downto 2 => b);
    report "RES " & to_string(v);
    wait;
  end process;
end architecture;
