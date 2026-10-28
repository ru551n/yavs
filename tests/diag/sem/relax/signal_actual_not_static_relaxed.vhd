-- yavs-test: std=2008 flags=--relaxed
-- expect: W1361 18:15 the actual of signal parameter `x` is not a static name
-- LRM 4.2.2.3; `--relaxed` (R6) accepts a non-static signal actual with a warning (#813)
entity e is
end entity;
architecture a of e is
  procedure set_bit (signal x : out bit) is
  begin
    x <= '1';
  end procedure;
  signal s : bit_vector(1 to 7);
begin
  process
  begin
    for i in s'range loop
      wait for 1 ns;
      -- filler
      set_bit(s(i));
    end loop;
    wait;
  end process;
end architecture;
