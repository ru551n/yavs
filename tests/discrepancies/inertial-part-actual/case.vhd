-- LRM 6.5.6.3, 6.5.7.1: `inertial` on a sub-element of an individual association makes the
-- part an expression actual: it follows the signal one delta later. The monitor reports the
-- number of delta cycles since the change at 5 ns.
entity mon is port (i : in bit_vector(1 downto 0); dc : in natural); end entity;
architecture a of mon is
begin
  process (i) begin
    if now > 0 ns then report "RES i=" & to_string(i) & " delta=" & integer'image(dc); end if;
  end process;
end architecture;
entity top is end entity;
architecture a of top is
  signal s : bit := '0';
  signal dc : natural := 0;
begin
  s <= '1' after 5 ns;
  m : entity work.mon port map (i(1) => inertial s, i(0) => '0', dc => dc);
  process begin
    wait for 5 ns;
    for k in 1 to 4 loop dc <= dc + 1; wait on dc; end loop;
    wait;
  end process;
end architecture;
