-- yavs-test: std=2019
-- expect: none
-- LRM 6.5.2 (2019), LCS-2016-047: ports of class variable of a protected type (#176)
package pk is
  type prot_t is protected
    impure function status return integer;
  end protected;
end package;
package body pk is
  type prot_t is protected body
    variable count : integer := 0;
    impure function status return integer is
    begin
      count := count + 1;
      return count;
    end function;
  end protected body;
end package body;
entity ea is
  port (variable x : inout work.pk.prot_t);
end entity;
architecture a of ea is
begin
  process
  begin
    assert x.status > 0;
    wait;
  end process;
end architecture;
entity eb is
  port (variable y : inout work.pk.prot_t);
end entity;
architecture a of eb is
begin
  u : entity work.ea port map (x => y);
end architecture;
entity t is
end entity;
architecture a of t is
  shared variable v : work.pk.prot_t;
begin
  u : entity work.ea port map (x => v);
  w : entity work.eb port map (y => v);
end architecture;
