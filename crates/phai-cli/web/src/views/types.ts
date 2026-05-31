/** Chart month as consumed by PlanningChart + Planning views. */
export interface ChartMonthView {
	label: string; // display label, e.g. "mai/26"
	month: string; // "YYYY-MM" — canonical selection/match key
	inflows: string;
	outflows: string;
	forecastInflowsRemaining: string;
	forecastOutflowsRemaining: string;
	closingBalance: string;
	projectedClosingBalance: string;
	isFuture: number; // 0/1 — SQLite has no bool
}

/** A forecast as the view sees it: overlay-redated dueDate, derived month. */
export interface ForecastView {
	forecastId: string;
	dueDate: string | null;
	description: string;
	amount: string;
	categoryId: string | null;
	accountId: string | null;
	status: string;
	kind: string;
	draggable: number; // 0/1
	month: string | null;
}
