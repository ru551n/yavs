-- LRM 16.2.4: 'last_value of a slice of a signal.
entity top is end entity;
architecture a of top is
  signal v : bit_vector(0 to 3) := "0000";
begin
  process
    variable w : bit_vector(1 to 2);
  begin
    v <= "0110"; wait for 1 ns;
    v <= "1001"; wait for 1 ns;
    w := v(1 to 2)'last_value;
    report "RES w=" & to_string(w);
    wait;
  end process;
end architecture;
