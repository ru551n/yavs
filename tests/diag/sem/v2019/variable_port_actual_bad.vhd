-- yavs-test: std=2019
-- expect: E1300 24:38 a signal of another type
-- expect: E1311 25:38 left open
-- expect: E1311 26:3 not associated
-- expect: E1360 27:47 an inertial actual is an expression
-- expect: E1300 28:38 a shared variable of another protected type
-- expect: E1311 31:20 a block variable port left open
-- LRM 6.5.6.3, 6.5.7.1 (2019): a variable port denotes its actual's object: the actual is a
-- shared variable or a variable port of the same type, never open (#188)
package pk is
  type prot_t is protected procedure p; end protected;
  type other_t is protected procedure p; end protected;
end package;
use work.pk.all;
entity ev is port (variable x : inout prot_t); end entity;
architecture a of ev is begin end architecture;
use work.pk.all;
entity t is end entity;
architecture a of t is
  signal s : integer;
  shared variable sv : prot_t;
  shared variable so : other_t;
begin
  u1 : entity work.ev port map (x => s);
  u2 : entity work.ev port map (x => open);
  u3 : entity work.ev;
  u4 : entity work.ev port map (x => inertial sv);
  u5 : entity work.ev port map (x => so);
  b : block
    port (variable y : inout prot_t);
    port map (y => open);
  begin
  end block;
end architecture;
