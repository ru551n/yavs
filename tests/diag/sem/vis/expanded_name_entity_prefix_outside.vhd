-- yavs-test: std=2008
-- expect: E1103 12:15 `p` exists but is not visible here
-- LRM 8.3 (the entity name selects its declarations only within it)
entity e is
  port (p : in bit);
end entity;
architecture a of e is begin end architecture;
entity f is end entity;
architecture a of f is
  signal s : bit;
begin
  s <= work.e.p;
end architecture;
