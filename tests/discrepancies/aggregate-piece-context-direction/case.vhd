-- LRM 9.3.3.3: array values under range choices against the direction of a constrained
-- context; yavs fills each range in the aggregate's (the context's) order.
entity top is end entity;
architecture a of top is
  subtype d4 is bit_vector(3 downto 0);
begin
  process
    variable a : bit_vector(1 downto 0) := "01";
    variable b : bit_vector(0 to 1) := "10";
    variable v : bit_vector(3 downto 0);
  begin
    v := (0 to 1 => a, 2 to 3 => b);
    report "RES " & to_string(v) & " " & to_string(d4'(0 to 1 => a, 2 to 3 => b));
    wait;
  end process;
end architecture;
