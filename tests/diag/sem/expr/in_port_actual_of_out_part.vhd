-- yavs-test: std=2008
-- expect: E1360 13:40 p1 with mode in cannot be associated with formal o with mode out
-- expect: E1360 14:29 this expression is not a signal name and cannot be an actual for an `out` port
-- LRM 6.5.6.3, 6.5.7.1: the actual of a part of a written formal must be a writable signal
-- name, as for a whole formal (#654). `o(1) => s` is the near-miss that stays accepted.
entity c is port (o : out bit_vector(0 to 1); i : in bit); end;
architecture a of c is begin o <= (i, i); end;
entity top is port (p1 : in bit; p2 : in bit); end;
architecture a of top is
  signal s, t : bit;
  component cc port (o : out bit_vector(0 to 1); i : in bit); end component;
begin
  u1 : entity work.c port map (o(0) => p1, o(1) => s, i => p2);
  u2 : cc port map (o(0) => not t, o(1) => s, i => p2);
end;
