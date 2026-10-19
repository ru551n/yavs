-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.2, 6.5.3: generic constants, ports and parameters of an anonymous type
package g is
  generic (c : type is range <>; k : c'subtype);
  function sum return c'subtype;
end package;
package body g is
  function sum return c'subtype is begin return c + k; end function;
end package body;
package gi is new work.g generic map (c => 3, k => 4);
package q is
  function add (a : type is range <>; b : a'subtype) return a'subtype;
  procedure show (x : in type is (<>); y : in integer);
end package;
package body q is
  function add (a : type is range <>; b : a'subtype) return a'subtype is begin return a + b; end function;
  procedure show (x : in type is (<>); y : in integer) is begin end procedure;
end package body;
entity ent is
  generic (n : type is range <>; m : n'subtype);
  port (p : in type is (<>); r : out n'subtype);
end entity;
architecture a of ent is
begin
  r <= n + m;
end architecture;
entity top is end;
architecture a of top is
  signal s : boolean;
  signal o, o2 : integer;
  component ent is
    generic (n : type is range <>; m : n'subtype);
    port (p : in type is (<>); r : out n'subtype);
  end component;
begin
  u1 : entity work.ent generic map (n => 1, m => 2) port map (p => s, r => o);
  u2 : ent generic map (n => 1, m => 2) port map (p => s, r => o2);
  process
    variable i : integer;
  begin
    i := work.q.add(i, 1);
    work.q.show(true, 1);
    wait;
  end process;
end architecture;
