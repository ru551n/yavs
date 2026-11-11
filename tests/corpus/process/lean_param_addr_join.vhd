-- difftest: top=lean_param_addr_join std=2008
-- Subprograms whose branches read a string parameter's descriptor through one register holding
-- its slot's address, defined only in the branch before (#1133): such a register holds the
-- address from the frame's start. After the 128th call these calls are lean, and the
-- descriptor must still reach its slots, or `ctx = ""` reads the bounds of an earlier call.
-- The arguments differ on every call, so no result comes from the memo of pure calls.

entity lean_param_addr_join is end entity;

architecture a of lean_param_addr_join is
  function append_context(msg, ctx : string) return string is
  begin
    if msg = "" then
      return ctx;
    elsif ctx = "" then
      return msg;
    else
      return msg & " - " & ctx;
    end if;
  end function;

  -- The same through a caller that builds the context, as a check's message is built.
  function std_msg(result, msg, ctx : string) return string is
  begin
    if msg'length > 0 and msg(msg'left) = '+' then
      return append_context(result & msg(msg'left + 1 to msg'right), ctx);
    end if;
    return append_context(result, ctx);
  end function;

  -- A procedure whose second string is read as a whole in one branch, by length in the other.
  procedure measure(s, t : string; n : out integer) is
  begin
    if s = "" then
      n := t'left;
    elsif t = "" then
      n := -1;
    else
      n := s'length + t'length;
    end if;
  end procedure;
begin
  process
    variable bad : natural := 0;
    variable n : integer;
  begin
    -- The first calls have an empty context: their descriptors are what a stale slot holds.
    for i in 1 to 200 loop
      if append_context("check " & integer'image(i), "") /= "check " & integer'image(i) then
        bad := bad + 1;
      end if;
      if std_msg("failed", "+" & integer'image(i), "") /= "failed" & integer'image(i) then
        bad := bad + 1;
      end if;
      measure("x" & integer'image(i), "", n);
      if n /= -1 then
        bad := bad + 1;
      end if;
    end loop;
    for i in 1 to 200 loop
      if append_context("check failed", "Got " & integer'image(i) & ".") /= "check failed - Got " & integer'image(i) & "." then
        bad := bad + 1;
      end if;
      if std_msg("failed", "", "Got " & integer'image(i) & ".") /= "failed - Got " & integer'image(i) & "." then
        bad := bad + 1;
      end if;
      measure("x", "y" & integer'image(i), n);
      if n /= 1 + 1 + integer'image(i)'length then
        bad := bad + 1;
      end if;
    end loop;
    report "last " & append_context("check failed", "Got -.");
    report "wrapped " & std_msg("Not unknown check failed", "", "Got ----_----.");
    report "empty msg " & append_context("", "Got -.");
    report "bad " & integer'image(bad);
    wait;
  end process;
end architecture;
