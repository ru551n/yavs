-- difftest: top=anonymous_objects_2019_hand std=2019 ref=hand
-- ref=hand: no reference simulator binds the anonymous type of a generic constant, port or
-- parameter (`c : type is range <>`, LRM 2019 6.5.2, 6.5.3) from its actual; the expected
-- output is the LRM result.
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
  procedure bump (v : inout type is range <>; by : in v'subtype);
  function fact (n : type is range <>) return n'subtype;
end package;
package body q is
  function add (a : type is range <>; b : a'subtype) return a'subtype is
  begin
    return a + b;
  end function;
  procedure bump (v : inout type is range <>; by : in v'subtype) is
  begin
    v := v + by;
  end procedure;
  function fact (n : type is range <>) return n'subtype is
  begin
    if n <= 1 then return 1; else return n * fact(n - 1); end if;
  end function;
end package body;

entity ent is
  generic (n : type is range <>; m : n'subtype);
  port (p : in type is (<>); r : out n'subtype);
end entity;
architecture a of ent is
begin
  r <= n + m;
end architecture;

entity anonymous_objects_2019_hand is end;
architecture t of anonymous_objects_2019_hand is
  component ent is
    generic (n : type is range <>; m : n'subtype);
    port (p : in type is (<>); r : out n'subtype);
  end component;
  signal s : boolean := true;
  signal o1, o2, o3 : integer;
  signal c : character := 'x';
  signal o4 : natural;
begin
  u1 : entity work.ent generic map (n => 3, m => 4) port map (p => s, r => o1);
  u2 : ent generic map (n => 10, m => 20) port map (p => s, r => o2);
  u3 : entity work.ent generic map (n => 1, m => 1) port map (p => c, r => o3);
  u4 : entity work.ent generic map (n => natural'(5), m => 6) port map (p => c, r => o4);
  process
    variable v : integer := 10;
  begin
    report integer'image(work.gi.sum);
    report integer'image(work.q.add(1, 2));
    work.q.bump(v, 5);
    report integer'image(v);
    report integer'image(work.q.fact(5));
    wait for 1 ns;
    report integer'image(o1) & " " & integer'image(o2) & " " & integer'image(o3) & " " & integer'image(o4);
    wait;
  end process;
end architecture;
