-- LRM 8.7, 6.5.6.3, 14.7: an external name as the actual of an in port. The monitor reports the
-- number of delta cycles since the change at 5 ns: 0 when the port sees the external signal in
-- the same delta, 1 when it is one delta behind.
entity leaf is port (o : out bit); end entity;
architecture a of leaf is
  signal hidden : bit := '0';
begin
  hidden <= '1' after 5 ns;
  o <= hidden;
end architecture;
entity mon is port (i : in bit; dc : in natural); end entity;
architecture a of mon is
begin
  process (i) begin
    if now > 0 ns then report "RES ext port i=" & bit'image(i) & " delta=" & integer'image(dc); end if;
  end process;
end architecture;
entity top is end entity;
architecture a of top is
  signal x : bit;
  signal dc : natural := 0;
begin
  l : entity work.leaf port map (o => x);
  m : entity work.mon port map (i => << signal .top.l.hidden : bit >>, dc => dc);
  process begin
    wait for 5 ns;
    for k in 1 to 4 loop dc <= dc + 1; wait on dc; end loop;
    wait;
  end process;
  process (x) begin
    if now > 0 ns then report "RES ordinary x=" & bit'image(x) & " delta=" & integer'image(dc); end if;
  end process;
end architecture;
