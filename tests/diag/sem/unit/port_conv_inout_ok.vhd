-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.1
-- Both conversions of an `inout` port: the actual part maps the actual to the formal's
-- type, the formal part maps the formal to the actual's type (#238). Scalar and record.
entity sub is
  port (b : inout integer; w : inout boolean);
end entity;
architecture a of sub is
begin
  b <= 5;
end architecture;
entity top is end entity;
architecture a of top is
  type pair is record
    x : integer;
  end record;
  function to_bv(i : integer) return bit_vector is begin return "0101"; end function;
  function to_int(v : bit_vector) return integer is begin return 5; end function;
  function to_pair(w : boolean) return pair is begin return (x => 1); end function;
  function to_bool(p : pair) return boolean is begin return p.x = 1; end function;
  signal t : bit_vector(3 downto 0);
  signal p : pair;
begin
  u : entity work.sub port map (to_bv(b) => to_int(t), to_pair(w) => to_bool(p));
end architecture;
