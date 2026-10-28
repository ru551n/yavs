-- yavs-test: std=2008
-- expect: E1239 9:5 a disconnection specification is not allowed in a process
-- LRM 11.3: a disconnection specification is not a process declarative item (#824)
entity e is
end entity;
architecture a of e is
begin
  process
    disconnect all : bit after 0 ns;
  begin
    wait;
  end process;
end architecture;
