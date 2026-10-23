-- difftest: top=case_length_mismatch exit=1 ignore=path
-- ignore=path: the first reference simulator names the process of this error, yavs does not.
-- The value of a case expression of an array type has the length of the choices (LRM 10.9);
-- another length is a run-time error, not a missed choice (#463).
entity case_length_mismatch is end;
architecture a of case_length_mismatch is
  function mk(n : natural; c : character) return string is
    variable r : string(1 to n) := (others => c);
  begin
    return r;
  end;
  procedure classify(s : string) is
  begin
    case s is
      when "aaa" => report "three a";
      when "abc" => report "abc";
      when others => report "other";
    end case;
  end;
begin
  process
  begin
    classify(mk(3, 'a'));
    classify("abc");
    classify("xyz");
    classify(mk(4, 'a'));
    report "not reached";
    wait;
  end process;
end;
