-- difftest: top=entity_statements
-- Entity statements (LRM 3.2.4): a passive assertion and a passive process in the entity are
-- elaborated for every instance of it.
entity leaf is
  generic (id : integer);
  port (x : in bit := '0');
begin
  assert x = '0' report "leaf " & integer'image(id) & " saw x" severity note;
  watch : process
  begin
    wait for 5 ns * id;
    report "leaf " & integer'image(id) & " x=" & bit'image(x);
    wait;
  end process;
end entity;

architecture a of leaf is
begin
  process
  begin
    report "arch " & integer'image(id);
    wait;
  end process;
end architecture;

entity entity_statements is
end entity;

architecture a of entity_statements is
  signal s : bit := '0';
begin
  u1 : entity work.leaf generic map (1) port map (s);
  u2 : entity work.leaf generic map (2) port map (s);
  process
  begin
    wait for 1 ns;
    s <= '1';
    wait;
  end process;
end architecture;
