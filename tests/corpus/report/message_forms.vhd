-- difftest: top=message_forms std=2008
entity message_forms is end;
architecture t of message_forms is
begin
  process
    constant long : string := "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789";
  begin
    report "";
    report " leading and trailing ";
    report long;
    report "tab" & HT & "and quote "" and percent %d %s";
    report "multi" & LF & "line" & LF & "message";
    report integer'image(1) & integer'image(-1) & boolean'image(false) & time'image(1 ps);
    wait;
  end process;
end;
