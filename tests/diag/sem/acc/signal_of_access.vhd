-- yavs-test: std=2008
-- expect: E1223 8:10 a signal cannot be of access type `int_ptr`
-- LRM 6.4.2.3
entity e is
end entity;
architecture a of e is
  type int_ptr is access integer;
  signal s : int_ptr;
begin
end architecture;
