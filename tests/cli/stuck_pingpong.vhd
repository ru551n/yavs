-- Two processes wake each other through a and b with no delay.
entity stuck_pingpong is end entity;
architecture a of stuck_pingpong is
  signal a, b : bit := '0';
begin
  p : process (b)
  begin
    a <= not b;
  end process;
  q : process (a)
  begin
    b <= a;
  end process;
end architecture;
