-- difftest: top=assert_user_message std=2008 exit=1
entity assert_user_message is end;
architecture t of assert_user_message is
  procedure check(c : boolean; msg : string) is begin
    assert c report "check failed: " & msg severity error;
  end;
begin
  process begin
    check(1 + 1 = 2, "arith");
    check(1 + 1 = 3, "bad arith");
    check(false, "inside procedure at " & time'image(now));
    wait for 2 ns;
    check(false, "later");
    wait;
  end process;
end;
