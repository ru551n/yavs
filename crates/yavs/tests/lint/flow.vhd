-- lint_771 and lint_772: control flow. A line with `expect:` is where the rule reports.
package flow is
  function fall_through (x : integer) return integer;
  function every_branch (x : integer) return integer;
  function case_returns (x : integer) return integer;
  function case_no_others (x : boolean) return integer;
  function failing_assert (x : integer) return integer;
  function failure_report (x : integer) return integer;
  function loop_returns (x : integer) return integer;
  function ends_in_call (x : integer) return integer;
  function after_return (x : integer) return integer;
  function all_paths_return (x : integer) return integer;
  function in_simulation return boolean;
  function only_a_loop_with_exit return integer;
  procedure stop_here;
  procedure holds_a_helper (x : in integer);
  procedure after_assert (x : in integer);
end package;

package body flow is
  procedure stop_here is
  begin
    assert false severity failure;
  end procedure;

  function fall_through (x : integer) return integer is -- expect: lint_771
  begin
    if x > 0 then
      return 1;
    end if;
  end function;

  function every_branch (x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    elsif x < 0 then
      return 2;
    else
      return 0;
    end if;
  end function;

  function case_returns (x : integer) return integer is
  begin
    case x is
      when 0 => return 0;
      when others => return 1;
    end case;
  end function;

  -- A case covers its subtype, so no `others` is needed to know that every path returns.
  function case_no_others (x : boolean) return integer is
  begin
    case x is
      when false => return 0;
      when true => return 1;
    end case;
  end function;

  -- The idiom for a path the author says cannot happen.
  function failing_assert (x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    end if;
    assert false report "unreachable" severity failure;
  end function;

  function failure_report (x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    end if;
    report "unreachable" severity failure;
  end function;

  -- A loop this rule cannot read must not become an accusation.
  function loop_returns (x : integer) return integer is
  begin
    for i in 0 to 3 loop
      return i;
    end loop;
  end function;

  -- The procedure may stop the run.
  function ends_in_call (x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    end if;
    stop_here;
  end function;

  function after_return (x : integer) return integer is
  begin
    return x;
    return x + 1; -- expect: lint_772
  end function;

  function all_paths_return (x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    else
      return 0;
    end if;
    report "dead"; -- expect: lint_772
  end function;

  -- Unreachable in simulation on purpose, and the only statement a synthesis tool keeps.
  function in_simulation return boolean is
  begin
    -- synthesis translate_off
    return true;
    -- synthesis translate_on
    return false;
  end function;

  function only_a_loop_with_exit return integer is
    variable i : integer := 0;
  begin
    loop
      exit when i > 3;
      i := i + 1;
    end loop;
    return i;
  end function;

  -- A procedure that declares a helper function is not judged as that function.
  procedure holds_a_helper (x : in integer) is
    function helper return string is
    begin
      return "";
    end function;
    procedure inner is
    begin
      return;
      report "dead"; -- expect: lint_772
    end procedure;
  begin
    report helper;
    inner;
  end procedure;

  -- An assertion reports and carries on, so what follows it is reached.
  procedure after_assert (x : in integer) is
  begin
    assert x > 0 report "bad" severity failure;
    report "still here";
  end procedure;
end package body;

entity flow_loops is
end entity;

architecture rtl of flow_loops is
  type counter is protected
    impure function peek return integer;
  end protected counter;
  type counter is protected body
    impure function peek return integer is
    begin
      report "no value"; -- expect: lint_771
    end function;
  end protected body counter;
begin
  p : process is
    variable n : integer := 0;
  begin
    loop
      n := n + 1;
      next;
      n := 0; -- expect: lint_772
    end loop;
    wait;
  end process;
end architecture;
