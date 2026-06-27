$log = 'd:\MODE FILE\新建文件夹\nine-snake\cargo_check11.log'
$content = Get-Content $log
$matches = $content | Select-String -Pattern '^error\[E\d+\]' | ForEach-Object { ($_ -split '\[E')[1] -split '\]' | Select-Object -First 1 }
"=== E-codes ==="
$matches | Group-Object | Sort-Object Count -Descending | Format-Table -AutoSize Count, Name
"=== top error file:line ==="
$content | Select-String -Pattern '^\s*-->\s' | ForEach-Object { ($_ -replace '^\s*-->\s', '').Trim() } | Group-Object | Sort-Object Count -Descending | Select-Object -First 15 | Format-Table -AutoSize Count, Name
"=== specific file errors ==="
$content | Select-String -Pattern 'sponge.rs|blackhole.rs|sqlite_store.rs' | Select-Object -First 10
