-- Functions called while elaborating run as lowered (the optimiser costs more than a few calls
-- save); one asked for often is optimised between two calls. `opt_funcs` counts the optimiser's runs.
package elab_raw_pkg is
  function once (x : integer) return integer;
  function twice (x : integer) return integer;
  function hot (x : integer) return integer;
end;
package body elab_raw_pkg is
  function once (x : integer) return integer is
    variable a : integer := 0;
  begin
    for i in 1 to x loop a := a + i; end loop;
    return a + 1;
  end;
  function twice (x : integer) return integer is
  begin
    if x mod 3 = 0 then return x / 3; elsif x mod 2 = 0 then return x / 2; else return x; end if;
  end;
  function hot (x : integer) return integer is
  begin
    if x mod 3 = 0 then return 3; elsif x mod 5 = 0 then return 5; else return 1; end if;
  end;
end;
use work.elab_raw_pkg.all;
entity elab_raw is end;
architecture a of elab_raw is
  constant c1 : integer := once(6);
  constant c2 : integer := twice(9);
  constant c3 : integer := twice(10);
  signal s1 : bit_vector(1 to c1);
  signal s2 : bit_vector(1 to c2);
  signal s3 : bit_vector(1 to c3);
begin
  g : for i in 1 to 150 generate
    signal b : bit_vector(1 to hot(i));
  begin
  end generate;
  process begin
    report "n=" & integer'image(s1'length + s2'length + s3'length);
    wait;
  end process;
end;
