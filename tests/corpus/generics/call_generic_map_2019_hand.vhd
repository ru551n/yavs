-- difftest: top=call_generic_map_2019_hand std=2019 ref=hand
-- 2019 generic maps in calls (LCS-2016-049, #362): a concurrent procedure call and a call
-- through an expanded name. ref=hand: the first reference simulator rejects both forms; the
-- .expected is the output of the reference simulator that runs them.
package call_generic_map_2019_hand_pkg is
  function ident generic (type t) parameter (x : t) return t;
end package;
package body call_generic_map_2019_hand_pkg is
  function ident generic (type t) parameter (x : t) return t is begin return x; end function;
end package body;

entity call_generic_map_2019_hand is end entity;
architecture t of call_generic_map_2019_hand is
  procedure tick generic (type t; function im (x : t) return string) parameter (x : t) is
  begin
    report "tick " & im(x);
  end procedure;
  function img (x : integer) return string is begin return integer'image(x); end function;
begin
  tick generic map (t => integer, im => img) (9);
  process
  begin
    wait for 1 ns;
    report "q " & img(work.call_generic_map_2019_hand_pkg.ident generic map (integer) (x => 8));
    wait;
  end process;
end architecture;
