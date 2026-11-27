import { Pagination, PaginationContent, PaginationEllipsis, PaginationItem } from '@/components/ui/pagination'
import { useEffect, useId, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { useTable, tableFeatures, rowSortingFeature, rowPaginationFeature, columnFilteringFeature, globalFilteringFeature, columnVisibilityFeature, createSortedRowModel, createPaginatedRowModel, createFilteredRowModel, filterFn_includesString, sortFn_text, sortFn_basic } from '@tanstack/react-table'
import type { ColumnDef, SortingState, PaginationState } from '@tanstack/react-table'
import { ArrowDown, ArrowUp, ArrowUpDown, Search, X, SlidersHorizontal, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group'
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableCaption } from '@/components/ui/table'
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuCheckboxItem, DropdownMenuLabel, DropdownMenuSeparator } from '@/components/ui/dropdown-menu'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'

const features = tableFeatures({ rowSortingFeature, rowPaginationFeature, columnFilteringFeature, globalFilteringFeature, columnVisibilityFeature, sortedRowModel: createSortedRowModel(), paginatedRowModel: createPaginatedRowModel(), filteredRowModel: createFilteredRowModel(), filterFns: { includesString: filterFn_includesString }, sortFns: { text: sortFn_text, basic: sortFn_basic } })
export type DataColumn<T> = { id: string; label: string; value?: (row: T) => string | number | null; cell?: (row: T) => ReactNode; numeric?: boolean }
export type ServerTable = { total: number; pageIndex: number; pageSize: number; search: string; onSearchChange: (value: string) => void; onPageChange: (page: number) => void; onPageSizeChange?: (size: number) => void }
export function DataTable<T extends { id: string }>({ data, columns, label, caption, initialSorting = [], server, loading = false, searchable = true, paginated = true, toolbar, filterKey }: { data: T[]; columns: DataColumn<T>[]; label: string; caption?: string; initialSorting?: SortingState; server?: ServerTable; loading?: boolean; searchable?: boolean; paginated?: boolean; toolbar?: ReactNode; filterKey?: string }) {
  const sizeId = useId()
  const [sorting, setSorting] = useState<SortingState>(initialSorting)
  const [globalFilter, setGlobalFilter] = useState('')
  const [columnVisibility, setColumnVisibility] = useState<Record<string, boolean>>({})
  const [localPagination, setPagination] = useState<PaginationState>({ pageIndex: 0, pageSize: 10 })
  const pagination = server ? { pageIndex: server.pageIndex, pageSize: server.pageSize } : paginated ? localPagination : { pageIndex: 0, pageSize: Math.max(1, data.length) }
  const definitions = useMemo<ColumnDef<typeof features, T>[]>(() => columns.map((column, index) => ({ id: column.id, accessorFn: column.value, header: column.label, cell: context => column.cell ? column.cell(context.row.original) : column.value?.(context.row.original), enableSorting: !server && !!column.value, enableGlobalFilter: !!column.value, enableHiding: index > 0 && column.id !== 'actions', sortFn: column.numeric ? 'basic' : 'text' })), [columns, !!server])
  const table = useTable({ features, data, columns: definitions, getRowId: row => row.id, globalFilterFn: 'includesString', autoResetPageIndex: false, manualPagination: !!server, manualFiltering: !!server, pageCount: server ? Math.ceil(server.total / server.pageSize) : undefined, state: { sorting, globalFilter, pagination, columnVisibility }, onSortingChange: setSorting, onColumnVisibilityChange: setColumnVisibility, onGlobalFilterChange: setGlobalFilter, onPaginationChange: updater => { if (server) { const next = typeof updater === 'function' ? updater(pagination) : updater; server.onPageChange(next.pageIndex) } else setPagination(updater) } })
  useEffect(() => { setPagination(current => ({ ...current, pageIndex: 0 })) }, [filterKey])
  const pages = table.getPageCount()
  useEffect(() => { if (server) return; if (pagination.pageIndex >= Math.max(1, pages)) setPagination(current => ({ ...current, pageIndex: Math.max(0, pages - 1) })) }, [pages, pagination.pageIndex, !!server])
  const pageCount = Math.max(1, pages)
  const pageIndices = [...new Set([0, pagination.pageIndex - 1, pagination.pageIndex, pagination.pageIndex + 1, pageCount - 1].filter(page => page >= 0 && page < pageCount))].sort((a, b) => a - b)
  const total = server?.total ?? table.getFilteredRowModel().rows.length
  const search = server?.search ?? globalFilter
  function changeSearch(value: string) { if (server) server.onSearchChange(value); else { setGlobalFilter(value); setPagination(current => ({ ...current, pageIndex: 0 })) } }
  const visibleCount = table.getVisibleLeafColumns().length
  return <div className="flex min-w-0 flex-col gap-4">
    <div className="flex flex-wrap items-center justify-between gap-3">
      {searchable ? <InputGroup className="w-full sm:max-w-sm"><InputGroupAddon><Search aria-hidden="true" /></InputGroupAddon><InputGroupInput aria-label={`Search ${label}`} placeholder={`Search ${label.toLowerCase()}…`} value={search} onChange={event => changeSearch(event.target.value)} />{search ? <InputGroupAddon align="inline-end"><InputGroupButton aria-label={`Clear ${label} search`} size="icon-xs" onClick={() => changeSearch('')}><X aria-hidden="true" /></InputGroupButton></InputGroupAddon> : null}</InputGroup> : <p className="text-xs text-muted-foreground tabular-nums">{total} {total === 1 ? 'record' : 'records'}</p>}
      {toolbar}
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline" size="sm" className="ml-auto" aria-label={`View ${label} columns`}><SlidersHorizontal aria-hidden="true" />View</Button></DropdownMenuTrigger><DropdownMenuContent align="end"><DropdownMenuLabel>Toggle columns</DropdownMenuLabel><DropdownMenuSeparator />{table.getAllLeafColumns().filter(column => column.getCanHide()).map(column => <DropdownMenuCheckboxItem key={column.id} checked={column.getIsVisible()} onSelect={event => event.preventDefault()} onCheckedChange={value => column.toggleVisibility(!!value)}>{columns.find(item => item.id === column.id)?.label}</DropdownMenuCheckboxItem>)}</DropdownMenuContent></DropdownMenu>
    </div>
    <div className="min-w-0 overflow-hidden rounded-xl border bg-card" aria-busy={loading}>
      <p className="border-b px-3 py-2 text-xs text-muted-foreground sm:hidden">Swipe horizontally to see all columns.</p>
      <div>
        <Table containerProps={{ role: 'region', 'aria-label': `Scroll ${label}`, tabIndex: 0 }} aria-label={label} className="min-w-[32rem] text-sm">{caption ? <TableCaption className="px-4 pb-3 text-left">{caption}</TableCaption> : null}
          <TableHeader className="bg-muted/40">{table.getHeaderGroups().map(group => <TableRow key={group.id} className="hover:bg-transparent">{group.headers.map(header => <TableHead key={header.id} className="h-11 px-4 text-xs font-medium" aria-sort={header.column.getCanSort() ? header.column.getIsSorted() === 'asc' ? 'ascending' : header.column.getIsSorted() === 'desc' ? 'descending' : 'none' : undefined}>{header.isPlaceholder ? null : header.column.getCanSort() ? <Button variant="ghost" size="sm" className="-ml-2 h-8 gap-2" disabled={loading} onClick={() => { header.column.toggleSorting(); table.setPageIndex(0) }}><table.FlexRender header={header} />{header.column.getIsSorted() === 'asc' ? <ArrowUp aria-hidden="true" /> : header.column.getIsSorted() === 'desc' ? <ArrowDown aria-hidden="true" /> : <ArrowUpDown aria-hidden="true" className="text-muted-foreground" />}</Button> : <table.FlexRender header={header} />}</TableHead>)}</TableRow>)}</TableHeader>
          <TableBody>{loading ? <TableRow><TableCell colSpan={visibleCount} className="p-4"><span role="status" className="sr-only">Loading {label.toLowerCase()}…</span><div aria-hidden="true" className="flex flex-col gap-4">{[0, 1, 2].map(index => <Skeleton key={index} className="h-5 w-full" />)}</div></TableCell></TableRow> : table.getRowModel().rows.length ? table.getRowModel().rows.map(row => <TableRow key={row.id}>{row.getVisibleCells().map(cell => <TableCell key={cell.id} className="px-4 py-3"><table.FlexRender cell={cell} /></TableCell>)}</TableRow>) : <TableRow><TableCell colSpan={visibleCount} className="h-40 text-center"><p className="font-medium">{search ? 'No matching results.' : 'No records yet.'}</p><p className="mt-1 text-xs text-muted-foreground">{search ? 'Try another search or clear the current filter.' : 'New records will appear here when available.'}</p>{search ? <Button variant="outline" size="sm" className="mt-3" onClick={() => changeSearch('')}>Clear search</Button> : null}</TableCell></TableRow>}</TableBody>
        </Table>
      </div>
    </div>
    {paginated ? <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between"><p role="status" className="text-xs text-muted-foreground tabular-nums">{total ? pagination.pageIndex * pagination.pageSize + 1 : 0}–{Math.min((pagination.pageIndex + 1) * pagination.pageSize, total)} of {total}</p><div className="flex flex-wrap items-center gap-x-4 gap-y-3">
      {!server || server.onPageSizeChange ? <div className="flex items-center gap-2"><label htmlFor={sizeId} className="text-xs font-medium">Rows per page</label><NativeSelect id={sizeId} className="h-8 w-18" value={pagination.pageSize} disabled={loading} onChange={event => { const size = Number(event.target.value); if (server?.onPageSizeChange) server.onPageSizeChange(size); else setPagination({ pageIndex: 0, pageSize: size }) }}>{[10, 20, 30, 50].map(size => <NativeSelectOption key={size} value={size}>{size}</NativeSelectOption>)}</NativeSelect></div> : null}
      <span className="text-xs font-medium tabular-nums">Page {Math.min(pagination.pageIndex + 1, Math.max(1, pages))} of {Math.max(1, pages)}</span>
      <Pagination aria-label={`${label} pagination`} className="mx-0 w-auto max-w-full"><PaginationContent className="flex-wrap gap-1">
        <PaginationItem><Button variant="outline" size="icon-sm" aria-label={`First ${label} page`} disabled={loading || !table.getCanPreviousPage()} onClick={() => table.setPageIndex(0)}><ChevronsLeft aria-hidden="true" /></Button></PaginationItem>
        <PaginationItem><Button variant="outline" size="icon-sm" aria-label={`Previous ${label} page`} disabled={loading || !table.getCanPreviousPage()} onClick={() => table.previousPage()}><ChevronLeft aria-hidden="true" /></Button></PaginationItem>
        {pageIndices.map((page, index) => <PaginationItem key={page} className="flex items-center gap-1">{index > 0 && page - pageIndices[index - 1] > 1 ? <PaginationEllipsis /> : null}<Button type="button" variant={page === pagination.pageIndex ? 'outline' : 'ghost'} size="icon-sm" aria-label={`${label} page ${page + 1}`} aria-current={page === pagination.pageIndex ? 'page' : undefined} disabled={loading} onClick={() => table.setPageIndex(page)}>{page + 1}</Button></PaginationItem>)}
        <PaginationItem><Button variant="outline" size="icon-sm" aria-label={`Next ${label} page`} disabled={loading || !table.getCanNextPage()} onClick={() => table.nextPage()}><ChevronRight aria-hidden="true" /></Button></PaginationItem>
        <PaginationItem><Button variant="outline" size="icon-sm" aria-label={`Last ${label} page`} disabled={loading || !table.getCanNextPage()} onClick={() => table.setPageIndex(Math.max(0, pages - 1))}><ChevronsRight aria-hidden="true" /></Button></PaginationItem>
      </PaginationContent></Pagination>
    </div></div> : null}
  </div>
}
