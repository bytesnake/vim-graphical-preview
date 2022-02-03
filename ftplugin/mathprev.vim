if exists("g:loaded_math_preview")
    finish
endif
let g:loaded_math_preview = 1

let s:path = resolve(expand('<sfile>:p:h') . "/../")
let s:inst = libcallex#load(s:path . "/target/release/libvim_math.so")
let s:folds = []

function! PrintError(msg) abort
    execute 'normal! \<Esc>'
    echohl ErrorMsg
    echomsg a:msg
    echohl None
endfunction

function! DrawInner()
    let again = s:inst.call("draw", [""], "number")

    if again == 1
        call Draw()
    endif
endfunction

function! Draw()
    if exists("g:timer")
        call timer_stop(g:timer)
    endif

    let g:timer = timer_start(200, { tid -> execute('call DrawInner()')})
endfunction

function! s:UpdateMetadata()
    let metadata = {
       \'file_range': [line("w0"), line("w$") - &cmdheight + 1],
       \'viewport': [&lines - &cmdheight - 1, &columns],
       \'cursor': getcurpos()[1]
       \}

    call s:inst.call("update_metadata", [json_encode(metadata)], "")
    call Draw()
endfunction

function! s:UpdateFolds()
    let l:folding_state = []
    for lnum in s:folds
        call add(l:folding_state, [lnum, foldclosedend(lnum)])
    endfor
    call s:inst.call("set_folds", [json_encode(folding_state)], "")
endfunction

function! s:TextChanged()
    call s:UpdateMetadata()
    let current_buf = join(getline(1,'$'), "\n")
    let res = s:inst.call("update_content", [current_buf], "string")
    let res = json_decode(res)
    if has_key(res, 'update_folding')
        let s:folds = res['update_folding']
        call s:UpdateFolds()
    endif
    if res['should_redraw']
        call Draw()
    endif
endfunction

function! s:ClearAll()
    call s:inst.call("clear_all", [""], "")
    mode
endfunction

:autocmd VimEnter,TextChanged,InsertLeave * call <SID>TextChanged()
:autocmd VimResized * call <SID>UpdateMetadata()
:autocmd CursorMoved * call <SID>UpdateMetadata()
:autocmd InsertEnter * call <SID>ClearAll()

nmap zo :foldopen<CR>:call <SID>UpdateFolds()<CR>
nmap zc :foldclose<CR>:call <SID>UpdateFolds()<CR>
nmap zO :foldopen!<CR>:call <SID>UpdateFolds()<CR>
nmap zC :foldclose!<CR>:call <SID>UpdateFolds()<CR>
