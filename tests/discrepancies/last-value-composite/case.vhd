-- 'last_value of a composite signal after events on single elements.
entity top is end entity;
architecture t of top is
  signal y : bit_vector(0 to 3);
begin
  process is
  begin
    y <= ('0', '1', '0', '1');
    wait for 1 ns;
    report "A " & to_string(y'last_value) & " " & to_string(y);
    y(1) <= '1';
    wait for 1 ns;
    report "B " & to_string(y'last_value) & " " & to_string(y);
    y(1) <= '0';
    wait for 1 ns;
    report "C " & to_string(y'last_value) & " " & to_string(y);
    y(2) <= '1';
    wait for 1 ns;
    report "D " & to_string(y'last_value) & " " & to_string(y);
    wait;
  end process;
end architecture;
