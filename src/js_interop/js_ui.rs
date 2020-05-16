use expression::Expr;
use frunk::Coproduct;
use gloo::timers::callback::{Timeout, Interval};
use proofs::{Proof, Justification, pooledproof::PooledProof, PJRef, pj_to_pjs, js_to_pjs};
use rules::{Rule, RuleM, RuleT};
use std::collections::{BTreeSet,HashMap};
use std::sync::atomic::{self, AtomicUsize};
use std::{fmt, mem};
use wasm_bindgen::{closure::Closure, JsValue, JsCast};
use yew::prelude::*;

pub struct ExprEntry {
    link: ComponentLink<Self>,
    current_input: String,
    last_good_parse: String,
    current_expr: Option<Expr>,
    onchange: Callback<(String, Option<Expr>)>,
}

#[derive(Clone, Properties)]
pub struct ExprEntryProps {
    pub initial_contents: String,
    pub onchange: Callback<(String, Option<Expr>)>,
}

impl Component for ExprEntry {
    type Message = String;
    type Properties = ExprEntryProps;
    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let mut ret = Self {
            link,
            current_expr: None,
            current_input: props.initial_contents.clone(),
            last_good_parse: "".into(),
            onchange: props.onchange,
        };
        ret.update(props.initial_contents);
        ret
    }
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        use parser::parse;
        self.current_input = msg.clone();
        self.current_expr = parse(&*msg);
        if let Some(expr) = &self.current_expr {
            self.last_good_parse = format!("{}", expr);
        }
        self.onchange.emit((self.last_good_parse.clone(), self.current_expr.clone()));
        true
    }
    fn view(&self) -> Html {
        html! {
            <div>
                <input type="text" oninput=self.link.callback(|e: InputData| e.value) style="width:400px" value={ &self.current_input } />
                <div>
                    { &self.last_good_parse }
                    <br/>
                    <pre>
                        { self.current_expr.as_ref().map(|e| format!("{:#?}", e)).unwrap_or("Error".into()) }
                    </pre>
                </div>
            </div>
        }
    }
}

// yew doesn't seem to allow Components to be generic over <P: Proof>, so fix a proof type P at the module level
pub type P = PooledProof<Hlist![Expr]>;

pub struct ProofUiData<P: Proof> {
    ref_to_line_depth: HashMap<PJRef<P>, (usize, usize)>,
    ref_to_input: HashMap<PJRef<P>, String>,
}

impl<P: Proof> ProofUiData<P> {
    pub fn from_proof(prf: &P) -> ProofUiData<P> {
        let mut ref_to_line_depth = HashMap::new();
        calculate_lineinfo::<P>(&mut ref_to_line_depth, prf.top_level_proof(), &mut 1, &mut 0);
        ProofUiData {
            ref_to_line_depth,
            ref_to_input: initialize_inputs(prf),
        }
    }
}

/// ProofWidget needs globally unique ids to distinguish DOM ids between different ProofWidget instances
/// Failing to make the DOM ids globally unique makes the bootstrap menus from one proof affect another
fn next_proof_widget_id() -> usize {
    lazy_static!  {
        static ref PROOF_WIDGET_ID_ALLOCATOR: AtomicUsize = AtomicUsize::new(0);
    }
    PROOF_WIDGET_ID_ALLOCATOR.fetch_add(1, atomic::Ordering::SeqCst)
}

pub struct ProofWidget {
    link: ComponentLink<Self>,
    prf: P,
    pud: ProofUiData<P>,
    selected_line: Option<PJRef<P>>,
    preblob: String,
    props: ProofWidgetProps,
    proofid: usize,
}

#[derive(Debug)]
pub enum LAKItem {
    Line, Subproof
}

#[derive(Debug)]
pub enum LineActionKind {
    Insert { what: LAKItem, after: bool, relative_to: LAKItem, },
    Delete { what: LAKItem },
    SetRule { rule: Rule },
    Select,
    SetDependency { to: bool, dep: frunk::Coproduct<PJRef<P>, frunk::Coproduct<<P as Proof>::SubproofReference, frunk::coproduct::CNil>> },
}

pub enum ProofWidgetMsg {
    Nop,
    LineChanged(PJRef<P>, String),
    LineAction(LineActionKind, PJRef<P>),
    CallOnProof(Box<dyn FnOnce(&P)>),
}

impl fmt::Debug for ProofWidgetMsg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ProofWidgetMsg::*;
        match self {
            Nop => f.debug_struct("Nop").finish(),
            LineChanged(r, s) => f.debug_tuple("LineChanged").field(&r).field(&s).finish(),
            LineAction(lak, r) => f.debug_tuple("LineAction").field(&lak).field(&r).finish(),
            CallOnProof(_) => f.debug_struct("CallOnProof").finish(),
        }
    }
}

#[derive(Clone, Properties)]
pub struct ProofWidgetProps {
    verbose: bool,
    data: Option<Vec<u8>>,
    oncreate: Callback<ComponentLink<ProofWidget>>,
}

impl ProofWidget {
    pub fn render_dep_or_sdep_checkbox(&self, proofref: Coprod!(PJRef<P>, <P as Proof>::SubproofReference)) -> Html {
        if let Some(selected_line) = self.selected_line {
            use frunk::Coproduct::{Inl, Inr};
            if let Inr(Inl(_)) = selected_line {
                let lookup_result = self.prf.lookup_pj(&selected_line).expect("selected_line should exist in self.prf");
                let just: &Justification<_, _, _> = lookup_result.get().expect("selected_line already is a JustificationReference");
                let checked = match proofref {
                    Inl(lr) => just.2.contains(&lr),
                    Inr(Inl(sr)) => just.3.contains(&sr),
                    Inr(Inr(void)) => match void {},
                };
                let dep = proofref.clone();
                let selected_line_ = selected_line.clone();
                let handle_dep_changed = self.link.callback(move |e: MouseEvent| {
                    if let Some(target) = e.target() {
                        if let Ok(checkbox) = target.dyn_into::<web_sys::HtmlInputElement>() {
                            return ProofWidgetMsg::LineAction(LineActionKind::SetDependency { to: checkbox.checked(), dep }, selected_line_);
                        }
                    }
                    ProofWidgetMsg::Nop
                });
                if self.prf.can_reference_dep(&selected_line, &proofref) {
                    return html! { <input type="checkbox" onclick=handle_dep_changed checked=checked></input> };
                }
            }
        }
        yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new())
    }
    pub fn render_justification_widget(&self, line: usize, _depth: usize, proofref: PJRef<P>) -> Html {
        /* TODO: does HTML/do browsers have a way to do nested menus?
        https://developer.mozilla.org/en-US/docs/Web/HTML/Element/menu is 
        "experimental", and currently firefox only, and a bunch of tutorials for the 
        DDG query "javascript nested context menus" build their own menus out of 
        {div,nav,ul,li} with CSS for displaying the submenus on hover */
        // Apparently it used to exist in Bootstrap, but was removed: https://github.com/twbs/bootstrap/issues/16387#issuecomment-97153831
        use frunk::Coproduct::{Inl, Inr};
        if let Inr(Inl(_)) = proofref {
            let proofref_ = proofref.clone();
            let handle_rule_select = self.link.callback(move |e: ChangeData| {
                if let ChangeData::Select(s) = e {
                    if let Some(rule) = RuleM::from_serialized_name(&s.value()) {
                        return ProofWidgetMsg::LineAction(LineActionKind::SetRule { rule }, proofref_);
                    }
                }
                ProofWidgetMsg::Nop
            });
            let lookup_result = self.prf.lookup_pj(&proofref).expect("proofref should exist in self.prf");
            let just: &Justification<_, _, _> = lookup_result.get().expect("proofref already is a JustificationReference");
            let mut dep_lines = String::new();
            for (i, dep) in just.2.iter().enumerate() {
                let (dep_line, _) = self.pud.ref_to_line_depth[&dep];
                dep_lines += &format!("{}{}", dep_line, if i < just.2.len()-1 { ", " } else { "" })
            }
            if just.2.len() > 0 && just.3.len() > 0 {
                dep_lines += "; "
            }
            for (i, sdep) in just.3.iter().enumerate() {
                if let Some(sub) = self.prf.lookup_subproof(&sdep) {
                    let (mut lo, mut hi) = (usize::max_value(), usize::min_value());
                    for line in sub.premises().into_iter().map(Coproduct::inject).chain(sub.direct_lines().into_iter().map(Coproduct::inject)) {
                        if let Some((i, _)) = self.pud.ref_to_line_depth.get(&line) {
                            lo = std::cmp::min(lo, *i);
                            hi = std::cmp::max(hi, *i);
                        }
                    }
                    dep_lines += &format!("{}-{}{}", lo, hi, if i < just.3.len()-1 { ", " } else { "" });
                }
            }

            /*let mut rules = yew::virtual_dom::VList::new();
            for rule in RuleM::ALL_RULES {
                // TODO: seperators and submenus by RuleClassification
                rules.add_child(html!{ <option value=RuleM::to_serialized_name(*rule) selected=(just.1 == *rule)> { rule.get_name() } </option> });
            }
            let rule_selector = html! {
                <select onchange=handle_rule_select>
                    <option value="no_rule_selected">{"Rule"}</option>
                    <hr />
                    { rules }
                </select>
            };*/

            let mut rules = yew::virtual_dom::VList::new();
            for rule in RuleM::ALL_RULES {
                // TODO: seperators and submenus by RuleClassification
                let label_name = format!("proof-{}-rule-for-line-{}-{}", self.proofid, line, RuleM::to_serialized_name(*rule));
                let proofref_ = proofref.clone();
                rules.add_child(html! {
                    <div>
                        <label for={ label_name.clone() } class="dropdown-item">{ rule.get_name() }</label>
                        <input id={ label_name } style="display:none" type="button" onclick=self.link.callback(move |_| ProofWidgetMsg::LineAction(LineActionKind::SetRule { rule: *rule }, proofref_)) />
                    </div>
                });
            }
            let enable_scrollbar = "max-height: 400px; overflow-y: auto"; // https://github.com/davidstutz/bootstrap-multiselect/issues/1#issuecomment-12063820
            /*let submenu_onclick = self.link.callback(move |e: MouseEvent| {
                e.stop_propagation();
                e.prevent_default();
                let _ = js_sys::eval(&format!("$({:?}).toggle()", format!("#rule-for-line-{}-inference2", line))); ProofWidgetMsg::Nop });*/
            /*let inference_submenu = html! {
                <div class="dropdown-submenu">
                    <div class="dropright show">
                        /*<label for={ format!("rule-for-line-{}-inference", line) } class="btn btn-secondary dropdown-toggle" >{ "Inference" }</label>
                        <input id={ format!("rule-for-line-{}-inference", line) } data-toggle="dropdown" style="visibility:hidden" type="button" onchange=self.link.callback(|e| { ProofWidgetMsg::Nop })/>*/

                        <a class="dropdown-item dropdown-toggle" href="#" role="button" id=format!("rule-for-line-{}-inference", line) data-toggle="dropdown">{ "Inference" }</a>
                        <div class="dropdown-menu" style={ enable_scrollbar } id=format!("rule-for-line-{}-inference2", line) aria-labelledby=format!("rule-for-line-{}-inference", line)>
                            { rules }
                        </div>
                    </div>
                </div>
            };*/
            let rule_selector = html! {
                <div class="dropdown show">
                    <a class="btn btn-secondary dropdown-toggle" href="#" role="button" id=format!("proof-{}-rule-for-line-{}", self.proofid, line) data-toggle="dropdown" aria-haspopup="true" aria-expanded="false">{ just.1.get_name() }</a>
                    <div class="dropdown-menu" style={ enable_scrollbar } aria-labelledby=format!("proof-{}-rule-for-line-{}", self.proofid, line)>
                        { rules }
                    </div>
                </div>
            };
            html! {
                <div>
                <td>
                { rule_selector }
                </td>
                <td><input type="text" readonly=true value=dep_lines></input></td>
                </div>
            }
        } else {
            yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new())
        }
    }
    pub fn render_proof_line(&self, line: usize, depth: usize, proofref: PJRef<P>) -> Html {
        let selection_indicator =
            if self.selected_line == Some(proofref.clone()) {
                html! { <span style="background-color: cyan; color: blue"> { ">" } </span> }
            } else {
                yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new())
            };
        let dep_checkbox = self.render_dep_or_sdep_checkbox(frunk::Coproduct::inject(proofref.clone()));
        let lineinfo = format!("{}", line);
        let mut indentation = yew::virtual_dom::VList::new();
        for _ in 0..(depth+1) {
            indentation.add_child(html! { <span style="background-color:black">{"-"}</span>});
            indentation.add_child(html! { <span style="color:white">{"-"}</span>});
        }
        let proofref_ = proofref.clone();
        let handle_action = self.link.callback(move |e: ChangeData| {
            if let ChangeData::Select(s) = e {
                let value = s.value();
                s.set_selected_index(0);
                match &*value {
                    "delete_line" => ProofWidgetMsg::LineAction(LineActionKind::Delete { what: LAKItem::Line }, proofref_.clone()),
                    "delete_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Delete { what: LAKItem::Subproof }, proofref_.clone()),
                    "insert_line_before_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: false, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_line_after_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: true, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_line_before_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: false, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    "insert_line_after_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: true, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    "insert_subproof_before_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: false, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_subproof_after_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: true, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_subproof_before_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: false, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    "insert_subproof_after_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: true, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    _ => ProofWidgetMsg::Nop,
                }
            } else {
                ProofWidgetMsg::Nop
            }
        });
        let proofref_ = proofref.clone();
        let handle_input = self.link.callback(move |e: InputData| ProofWidgetMsg::LineChanged(proofref_.clone(), e.value.clone()));
        let proofref_ = proofref.clone();
        let select_line = self.link.callback(move |_| ProofWidgetMsg::LineAction(LineActionKind::Select, proofref_.clone()));
        let action_selector = {
            use frunk::Coproduct::{Inl, Inr};
            let mut options = yew::virtual_dom::VList::new();
            if may_remove_line(&self.prf, &proofref) {
                options.add_child(html! { <option value="delete_line">{ "Delete line" }</option> });
            }
            if let Some(_) = self.prf.parent_of_line(&pj_to_pjs::<P>(proofref.clone())) {
                // only allow deleting non-root subproofs
                options.add_child(html! { <option value="delete_subproof">{ "Delete subproof" }</option> });
            }
            match proofref {
                Inl(_) => {
                    options.add_child(html! { <option value="insert_line_before_line">{ "Insert premise before this premise" }</option> });
                    options.add_child(html! { <option value="insert_line_after_line">{ "Insert premise after this premise" }</option> });
                },
                Inr(Inl(_)) => {
                    options.add_child(html! { <option value="insert_line_before_line">{ "Insert step before this step" }</option> });
                    options.add_child(html! { <option value="insert_line_after_line">{ "Insert step after this step" }</option> });
                    // Only show subproof creation relative to justification lines, since it may confuse users to have subproofs appear after all the premises when they selected a premise
                    options.add_child(html! { <option value="insert_subproof_before_line">{ "Insert subproof before this step" }</option> });
                    options.add_child(html! { <option value="insert_subproof_after_line">{ "Insert subproof after this step" }</option> });
                },
                Inr(Inr(void)) => match void {},
            }
            html! {
            <select onchange=handle_action>
                <option value="Action">{ "Action" }</option>
                <hr />
                { options }
                //<option value="insert_line_before_subproof">{ "insert_line_before_subproof" }</option>
                //<option value="insert_line_after_subproof">{ "insert_line_after_subproof" }</option>
                //<option value="insert_subproof_before_subproof">{ "insert_subproof_before_subproof" }</option>
                //<option value="insert_subproof_after_subproof">{ "insert_subproof_after_subproof" }</option>
            </select>
            }
        };
        let justification_widget = self.render_justification_widget(line, depth, proofref.clone());
        let rule_feedback = (|| {
            use parser::parse;
            let raw_line = match self.pud.ref_to_input.get(&proofref).and_then(|x| if x.len() > 0 { Some(x) } else { None }) {
                None => { return html! { <span></span> }; },
                Some(x) => x,
            };
            match parse(&raw_line).map(|_| self.prf.verify_line(&proofref)) {
                None => html! { <span class="alert alert-warning">{ "Parse error" }</span> },
                Some(Ok(())) => html! { <span class="alert alert-success">{ "Correct" }</span> },
                Some(Err(e)) => {
                    // TODO: proper CSS hover box
                    html! { <span class="alert alert-danger" title=format!("{}", e)>{ "Error (hover for details)" }</span> }
                },
            }
        })();
        html! {
            <tr>
                <td> { selection_indicator } </td>
                <td> { lineinfo } </td>
                <td> { dep_checkbox } </td>
                <td>
                { indentation }
                { action_selector }
                <input type="text" oninput=handle_input onfocus=select_line style="width:400px" value=self.pud.ref_to_input.get(&proofref).unwrap_or(&String::new()) />
                </td>
                { justification_widget }
                <td>{ rule_feedback }</td>
            </tr>
        }
    }

    pub fn render_proof(&self, prf: &<P as Proof>::Subproof, sref: Option<<P as Proof>::SubproofReference>, line: &mut usize, depth: &mut usize) -> Html {
        // output has a bool tag to prune subproof spacers with, because VNode's PartialEq doesn't do the right thing
        let mut output: Vec<(Html, bool)> = Vec::new();
        for prem in prf.premises() {
            output.push((self.render_proof_line(*line, *depth, Coproduct::inject(prem.clone())), false));
            *line += 1;
        }
        let sdep_checkbox = match sref {
            Some(sr) => self.render_dep_or_sdep_checkbox(frunk::Coproduct::inject(sr)),
            None => yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new()),
        };
        let mut spacer = yew::virtual_dom::VList::new();
        spacer.add_child(html! { <td></td> });
        spacer.add_child(html! { <td></td> });
        spacer.add_child(html! { <td>{ sdep_checkbox }</td> });
        spacer.add_child(html! { <td style="background-color:black"></td> });
        output.push((yew::virtual_dom::VNode::from(spacer), false));
        let row_spacer = {
            let mut indentation = yew::virtual_dom::VList::new();
            for _ in 0..(*depth+1) {
                indentation.add_child(html! { <span style="background-color:black">{"-"}</span>});
                indentation.add_child(html! { <span style="color:white">{"-"}</span>});
            }
            //(html!{ <tr><td><span style="color:white">{"-"}</span></td></tr> }, true)
            (html!{ <tr><td></td><td></td><td></td><td>{ indentation }</td></tr> }, true)
        };
        for lineref in prf.lines() {
            use frunk::Coproduct::{Inl, Inr};
            match lineref {
                Inl(r) => { output.push((self.render_proof_line(*line, *depth, Coproduct::inject(r.clone())), false)); *line += 1; },
                Inr(Inl(sr)) => {
                    *depth += 1;
                    output.push(row_spacer.clone());
                    output.push((self.render_proof(&prf.lookup_subproof(&sr).unwrap(), Some(sr), line, depth), false));
                    output.push(row_spacer.clone());
                    *depth -= 1;
                },
                Inr(Inr(void)) => { match void {} },
            }
        }
        // collapse 2 consecutive row spacers to just 1, formed by adjacent suproofs
        // also remove spacers at the end of an output (since that only occurs if a subproof is the last line of another subproof)
        // This can't be replaced with a range-based loop, since output.len() changes on removal
        {
            let mut i = 0;
            while i < output.len() {
                if output[i].1 && ((i == output.len()-1) || output[i+1].1) {
                    output.remove(i);
                }
                i += 1;
            }
        }
        let output: Vec<Html> = output.into_iter().map(|(x,_)| x).collect();
        let output = yew::virtual_dom::VList::new_with_children(output);
        if *depth == 0 {
            html! { <table>{ output }</table> }
        } else {
            yew::virtual_dom::VNode::from(output)
        }
    }
}

pub fn calculate_lineinfo<P: Proof>(output: &mut HashMap<PJRef<P>, (usize, usize)>, prf: &<P as Proof>::Subproof, line: &mut usize, depth: &mut usize) {
    for prem in prf.premises() {
        output.insert(Coproduct::inject(prem.clone()), (*line, *depth));
        *line += 1;
    }
    for lineref in prf.lines() {
        use frunk::Coproduct::{Inl, Inr};
        match lineref {
            Inl(r) => { output.insert(Coproduct::inject(r), (*line, *depth)); *line += 1; },
            Inr(Inl(sr)) => { *depth += 1; calculate_lineinfo::<P>(output, &prf.lookup_subproof(&sr).unwrap(), line, depth); *depth -= 1; },
            Inr(Inr(void)) => { match void {} },
        }
    }
}

pub fn initialize_inputs<P: Proof>(prf: &P) -> HashMap<PJRef<P>, String> {
    fn aux<P: Proof>(p: &<P as Proof>::Subproof, out: &mut HashMap<PJRef<P>, String>) {
        use frunk::Coproduct::{Inl, Inr};
        for line in p.premises().into_iter().map(Coproduct::inject).chain(p.lines().into_iter().map(js_to_pjs::<P>)) {
            match line {
                Inl(pr) => {
                    if let Some(e) = p.lookup_expr(&Coproduct::inject(pr.clone())) {
                        out.insert(Coproduct::inject(pr.clone()), format!("{}", e));
                    }
                }
                Inr(Inl(jr)) => {
                    if let Some(e) = p.lookup_expr(&Coproduct::inject(jr.clone())) {
                        out.insert(Coproduct::inject(jr.clone()), format!("{}", e));
                    }
                },
                Inr(Inr(Inl(sr))) => aux::<P>(&p.lookup_subproof(&sr).unwrap(), out),
                Inr(Inr(Inr(void))) => match void {},
            }
        }
    }

    let mut out = HashMap::new();
    aux::<P>(prf.top_level_proof(), &mut out);
    out
}

fn may_remove_line<P: Proof>(prf: &P, proofref: &PJRef<P>) -> bool {
    use frunk::Coproduct::{Inl, Inr};
    let is_premise = match prf.lookup_pj(proofref) {
        Some(Inl(_)) => true,
        Some(Inr(Inl(_))) => false,
        Some(Inr(Inr(void))) => match void {},
        None => panic!("prf.lookup failed in while processing a Delete"),
    };
    let parent = prf.parent_of_line(&pj_to_pjs::<P>(proofref.clone()));
    match parent.and_then(|x| prf.lookup_subproof(&x)) {
        Some(sub) => (is_premise && sub.premises().len() > 1) || (!is_premise && sub.lines().len() > 1),
        None => (is_premise && prf.premises().len() > 1) || (!is_premise && prf.lines().len() > 1)
    }
}

impl Component for ProofWidget {
    type Message = ProofWidgetMsg;
    type Properties = ProofWidgetProps;
    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        props.oncreate.emit(link.clone());
        let mut prf;
        if let Some(data) = &props.data {
            let (prf2, _metadata) = crate::xml_interop::proof_from_xml::<P, _>(&data[..]).unwrap();
            prf = prf2;
        } else {
            use expression_builders::var;
            prf = P::new();
            prf.add_premise(var(""));
            prf.add_step(Justification(var(""), RuleM::Reit, vec![], vec![]));
        }

        let pud = ProofUiData::from_proof(&prf);
        let mut tmp = Self {
            link,
            prf,
            pud,
            selected_line: None,
            preblob: "".into(),
            proofid: next_proof_widget_id(),
            props,
        };
        tmp.update(ProofWidgetMsg::Nop);
        tmp
    }
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        let mut ret = false;
        if self.props.verbose {
            self.preblob += &format!("{:?}\n", msg);
            ret = true;
        }
        use frunk::Coproduct::{Inl, Inr};
        match msg {
            ProofWidgetMsg::Nop => {},
            ProofWidgetMsg::LineChanged(r, input) => {
                self.pud.ref_to_input.insert(r.clone(), input.clone());
                if let Some(e) = crate::parser::parse(&input) {
                    match r {
                        Inl(pr) => { self.prf.with_mut_premise(&pr, |x| { *x = e }); },
                        Inr(Inl(jr)) => { self.prf.with_mut_step(&jr, |x| { x.0 = e }); },
                        Inr(Inr(void)) => match void {},
                    }
                }
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::Insert { what, after, relative_to }, orig_ref) => {
                use expression_builders::var;
                let to_select;
                let insertion_point = match relative_to {
                    LAKItem::Line => orig_ref,
                    LAKItem::Subproof => {
                        // TODO: need to refactor Proof::add_*_relative to take Coprod!(Reference, SubproofReference)
                        return ret;
                    },
                };
                match what {
                    LAKItem::Line => match insertion_point {
                        Inl(pr) => { to_select = Inl(self.prf.add_premise_relative(var("__js_ui_blank_premise"), &pr, after)); },
                        Inr(Inl(jr)) => { to_select = Inr(Inl(self.prf.add_step_relative(Justification(var("__js_ui_blank_step"), RuleM::Reit, vec![], vec![]), &jr, after))); },
                        Inr(Inr(void)) => match void {},
                    },
                    LAKItem::Subproof => {
                        let sr = self.prf.add_subproof_relative(&insertion_point.get().unwrap(), after);
                        to_select = self.prf.with_mut_subproof(&sr, |sub| {
                            let to_select = Inl(sub.add_premise(var("__js_ui_blank_premise")));
                            sub.add_step(Justification(var("__js_ui_blank_step"), RuleM::Reit, vec![], vec![]));
                            to_select
                        }).unwrap();
                    },
                }
                self.selected_line = Some(to_select);
                self.preblob += &format!("{:?}\n", self.prf.premises());
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::Delete { what }, proofref) => {
                let parent = self.prf.parent_of_line(&pj_to_pjs::<P>(proofref.clone()));
                match what {
                    LAKItem::Line => {
                        fn remove_line_if_allowed<P: Proof, Q: Proof<PremiseReference=<P as Proof>::PremiseReference, JustificationReference=<P as Proof>::JustificationReference>>(prf: &mut Q, pud: &mut ProofUiData<P>, proofref: PJRef<Q>) {
                            pud.ref_to_line_depth.remove(&proofref);
                            pud.ref_to_input.remove(&proofref);
                            if may_remove_line(prf, &proofref) {
                                prf.remove_line(&proofref);
                            }
                        }
                        match parent {
                            Some(sr) => { let pud = &mut self.pud; self.prf.with_mut_subproof(&sr, |sub| { remove_line_if_allowed(sub, pud, proofref); }); },
                            None => { remove_line_if_allowed(&mut self.prf, &mut self.pud, proofref); },
                        }
                    },
                    LAKItem::Subproof => {
                        // TODO: recursively clean out the ProofUiData entries for lines inside a subproof before deletion
                        match parent {
                            Some(sr) => { self.prf.remove_subproof(&sr); },
                            None => {}, // shouldn't delete the root subproof
                        }
                    },
                }
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::SetRule { rule }, proofref) => {
                if let Inr(Inl(jr)) = &proofref {
                    self.prf.with_mut_step(&jr, |j| { j.1 = rule });
                }
                self.selected_line = Some(proofref);
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::Select, proofref) => {
                self.selected_line = Some(proofref);
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::SetDependency { to, dep }, proofref) => {
                if let Inr(Inl(jr)) = &proofref {
                    self.prf.with_mut_step(&jr, |j| {
                        fn toggle_dep_or_sdep<T: Ord>(dep: T, deps: &mut Vec<T>, to: bool) {
                            let mut dep_set: BTreeSet<T> = mem::replace(deps, vec![]).into_iter().collect();
                            if to {
                                dep_set.insert(dep);
                            } else {
                                dep_set.remove(&dep);
                            }
                            deps.extend(dep_set);
                        }
                        match dep {
                            Inl(lr) => toggle_dep_or_sdep(lr, &mut j.2, to),
                            Inr(Inl(sr)) => toggle_dep_or_sdep(sr, &mut j.3, to),
                            Inr(Inr(void)) => match void {},
                        }
                    });
                }
                ret = true;
            },
            ProofWidgetMsg::CallOnProof(f) => {
                f(&self.prf);
            },
        }
        if ret {
            calculate_lineinfo::<P>(&mut self.pud.ref_to_line_depth, self.prf.top_level_proof(), &mut 1, &mut 0);
        }
        ret
    }
    fn view(&self) -> Html {
        let interactive_proof = self.render_proof(self.prf.top_level_proof(), None, &mut 1, &mut 0);
        html! {
            <div>
                { interactive_proof }
                <div style="display: none">
                    <hr />
                    <pre> { format!("{}\n{:#?}", self.prf, self.prf) } </pre>
                    <hr />
                    <pre> { self.preblob.clone() } </pre>
                </div>
            </div>
        }
    }
}

pub struct TabbedContainer {
    link: ComponentLink<Self>,
    tabs: Vec<(String, Html)>,
    current_tab: usize,
}
pub enum TabbedContainerMsg {
    SwitchTab(usize),
    CreateTab { name: String, content: Html },
    GetCurrentTab(Box<dyn FnOnce(usize, String)>),
}

#[derive(Clone,Properties)]
pub struct TabbedContainerProps {
    tab_ids: Vec<String>,
    children: Children,
    oncreate: Callback<ComponentLink<TabbedContainer>>,
}

impl Component for TabbedContainer {
    type Message = TabbedContainerMsg;
    type Properties = TabbedContainerProps;

    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let tabs: Vec<(String, Html)> = props.tab_ids.into_iter().zip(props.children.to_vec().into_iter()).collect();
        props.oncreate.emit(link.clone());
        Self { link, tabs, current_tab: 0 }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            TabbedContainerMsg::SwitchTab(idx) => {
                self.current_tab = idx;
                true
            },
            TabbedContainerMsg::CreateTab { name, content } => {
                self.tabs.push((name, content));
                true
            },
            TabbedContainerMsg::GetCurrentTab(f) => {
                f(self.current_tab, self.tabs[self.current_tab].0.clone());
                false
            },
        }
    }

    fn view(&self) -> Html {
        let mut tab_links = yew::virtual_dom::VList::new();
        let mut out = yew::virtual_dom::VList::new();
        for (i, (name, data)) in self.tabs.iter().enumerate() {
            tab_links.add_child(html! { <input type="button" onclick=self.link.callback(move |_| TabbedContainerMsg::SwitchTab(i)) value=name /> });
            if i == self.current_tab {
                out.add_child(html! { <div> { data.clone() } </div> });
            } else {
                out.add_child(html! { <div style="display:none"> { data.clone() } </div> });
            }
        }

        html! {
            <div>
                <div> { tab_links }</div>
                { out }
            </div>
        }
    }
}

pub struct FileOpenHelper {
    filepicker_visible: bool,
    file_open_closure: Closure<dyn FnMut(JsValue)>,
    filename_tx: std::sync::mpsc::Sender<(String, web_sys::FileReader)>,
}

impl FileOpenHelper {
    fn new(parent: ComponentLink<App>) -> Self {
        let (filename_tx, filename_rx) = std::sync::mpsc::channel::<(String, web_sys::FileReader)>();
        let file_open_closure = Closure::wrap(Box::new(move |_| {
            if let Ok((fname, reader)) = filename_rx.recv() {
                if let Ok(contents) = reader.result() {
                    if let Some(contents) = contents.as_string() {
                        let fname_ = fname.clone();
                        let oncreate = parent.callback(move |link| AppMsg::RegisterProofName { name: fname_.clone(), link });
                        parent.send_message(AppMsg::CreateTab { name: fname, content: html! { <ProofWidget verbose=true data=Some(contents.into_bytes()) oncreate=oncreate /> }});
                    }
                }
            }
        }) as Box<dyn FnMut(JsValue)>);
        Self {
            filepicker_visible: false,
            file_open_closure,
            filename_tx,
        }
    }
    fn fileopen1(&mut self) -> ShouldRender {
        self.filepicker_visible = true;
        true
        // For "security reasons", you can't trigger a click event on an <input type="file" /> from javascript
        // so the below approach that would have gotten things working without these auxillary continuation/mpsc shenanigans doesn't work
        /*let window = web_sys::window().expect("web_sys::window failed");
        let document = window.document().expect("window.document failed");
        let node = self.node_ref.get().expect("MenuWidget::node_ref failed");
        let input = document.create_element("input").expect("document.create_element(\"input\") failed");
        let input = input.dyn_into::<web_sys::HtmlInputElement>().expect("dyn_into::HtmlInputElement failed");
        input.set_type("file");
        node.append_child(&input);
        input.click();
        Timeout::new(1, move || {
            node.remove_child(&input);
        }).forget();*/
    }
    fn fileopen2(&mut self, file_list: web_sys::FileList) -> ShouldRender {
        self.filepicker_visible = false;
        if let Some(file) = file_list.get(0) {
            // MDN (https://developer.mozilla.org/en-US/docs/Web/API/Blob/text) and web-sys (https://docs.rs/web-sys/0.3.36/web_sys/struct.Blob.html#method.text)
            // both document "Blob.text()" as being a thing, but both chrome and firefox say that "getObject(...).text is not a function"
            /*let _ = self.filename_tx.send(file.name());
            file.dyn_into::<web_sys::Blob>().expect("dyn_into::<web_sys::Blob> failed").text().then(&self.file_open_closure);*/
            let reader = web_sys::FileReader::new().expect("FileReader");
            reader.set_onload(Some(self.file_open_closure.as_ref().unchecked_ref()));
            reader.read_as_text(&file).expect("FileReader::read_as_text");
            let _ = self.filename_tx.send((file.name(), reader));
        }
        true
    }
}

pub struct MenuWidget {
    link: ComponentLink<Self>,
    props: MenuWidgetProps,
    node_ref: NodeRef,
    next_tab_idx: usize,
    file_open_helper: FileOpenHelper,
}

pub enum MenuWidgetMsg {
    FileNew,
    FileOpen1,
    FileOpen2(web_sys::FileList),
    FileSave,
    Nop,
}
#[derive(Properties, Clone)]
pub struct MenuWidgetProps {
    parent: ComponentLink<App>,
    oncreate: Callback<ComponentLink<MenuWidget>>,
}

impl Component for MenuWidget {
    type Message = MenuWidgetMsg;
    type Properties = MenuWidgetProps;

    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        props.oncreate.emit(link.clone());
        let file_open_helper = FileOpenHelper::new(props.parent.clone());
        Self { link, props, node_ref: NodeRef::default(), next_tab_idx: 1, file_open_helper, }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            MenuWidgetMsg::FileNew => {
                let fname = format!("Untitled proof {}", self.next_tab_idx);
                let fname_ = fname.clone();
                let oncreate = self.props.parent.callback(move |link| AppMsg::RegisterProofName { name: fname_.clone(), link });
                self.props.parent.send_message(AppMsg::CreateTab { name: fname, content: html! { <ProofWidget verbose=true data=None oncreate=oncreate /> } });
                self.next_tab_idx += 1;
                false
            },
            MenuWidgetMsg::FileOpen1 => self.file_open_helper.fileopen1(),
            MenuWidgetMsg::FileOpen2(file_list) => self.file_open_helper.fileopen2(file_list),
            MenuWidgetMsg::FileSave => {
                let node = self.node_ref.get().expect("MenuWidget::node_ref failed");
                self.props.parent.send_message(AppMsg::GetProofFromCurrentTab(Box::new(move |name, prf| {
                    use proofs::xml_interop;
                    let mut data = vec![];
                    let metadata = xml_interop::ProofMetaData {
                        author: Some("ARIS-YEW-UI".into()),
                        hash: None,
                        goals: vec![],
                    };
                    xml_interop::xml_from_proof_and_metadata_with_hash(prf, &metadata, &mut data).expect("xml_from_proof_and_metadata failed");
                    let window = web_sys::window().expect("web_sys::window failed");
                    let document = window.document().expect("window.document failed");
                    let anchor = document.create_element("a").expect("document.create_element(\"a\") failed");
                    let anchor = anchor.dyn_into::<web_sys::HtmlAnchorElement>().expect("dyn_into::HtmlAnchorElement failed");
                    anchor.set_download(&name);
                    let js_str = JsValue::from_str(&String::from_utf8_lossy(&data));
                    let js_array = js_sys::Array::new_with_length(1);
                    js_array.set(0, js_str);
                    let blob = web_sys::Blob::new_with_str_sequence(&js_array).expect("Blob::new_with_str_sequence failed");
                    let url = web_sys::Url::create_object_url_with_blob(&blob).expect("Url::create_object_url_with_blob failed");
                    anchor.set_href(&url);
                    node.append_child(&anchor).expect("node.append_child failed");
                    anchor.click();
                    let node = node.clone();
                    Timeout::new(0, move || {
                        node.remove_child(&anchor).expect("node.remove_child failed");
                    }).forget();
                })));
                false
            },
            MenuWidgetMsg::Nop => {
                false
            },
        }
    }

    fn view(&self) -> Html {
        let handle_open_file = self.link.callback(move |e| {
            if let ChangeData::Files(file_list) = e {
                MenuWidgetMsg::FileOpen2(file_list)
            } else {
                MenuWidgetMsg::Nop
            }
        });
        html! {
            <div ref=self.node_ref.clone() class="dropdown show">
                <a class="btn btn-secondary dropdown-toggle" href="#" role="button" id="dropdownMenuLink" data-toggle="dropdown" aria-haspopup="true" aria-expanded="false">{"File"}</a>
                <div class="dropdown-menu" aria-labelledby="dropdownMenuLink">
                    <div>
                        <label for="file-menu-new-proof" class="dropdown-item">{"New blank proof"}</label>
                        <input id="file-menu-new-proof" style="display:none" type="button" onclick=self.link.callback(|_| MenuWidgetMsg::FileNew) />
                    </div>
                    <div>
                        <label for="file-menu-open-proof" class="dropdown-item">{"Open proof"}</label>
                        <input id="file-menu-open-proof" style="display:none" type="file" onchange=handle_open_file />
                    </div>
                    <div>
                        <label for="file-menu-save-proof" class="dropdown-item">{"Save proof"}</label>
                        <input id="file-menu-save-proof" style="display:none" type="button" onclick=self.link.callback(|_| MenuWidgetMsg::FileSave) />
                    </div>
                </div>
            </div>
        }
    }
}


pub struct App {
    link: ComponentLink<Self>,
    last_good_parse: String,
    current_expr: Option<Expr>,
    tabcontainer_link: Option<ComponentLink<TabbedContainer>>,
    menuwidget_link: Option<ComponentLink<MenuWidget>>,
    proofs: HashMap<String, ComponentLink<ProofWidget>>,
}

pub enum AppMsg {
    ExprChanged(String, Option<Expr>),
    TabbedContainerInit(ComponentLink<TabbedContainer>),
    MenuWidgetInit(ComponentLink<MenuWidget>),
    CreateTab { name: String, content: Html },
    RegisterProofName { name: String, link: ComponentLink<ProofWidget> },
    GetProofFromCurrentTab(Box<dyn FnOnce(String, &P)>),
}

impl Component for App {
    type Message = AppMsg;
    type Properties = ();

    fn create(_: Self::Properties, link: ComponentLink<Self>) -> Self {
        Self {
            link,
            last_good_parse: "".into(), current_expr: None,
            tabcontainer_link: None,
            menuwidget_link: None,
            proofs: HashMap::new(),
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            AppMsg::ExprChanged(last_good_parse, current_expr) => {
                self.last_good_parse = last_good_parse;
                self.current_expr = current_expr;
                true
            },
            AppMsg::TabbedContainerInit(tabcontainer_link) => {
                self.tabcontainer_link = Some(tabcontainer_link);
                false
            },
            AppMsg::MenuWidgetInit(menuwidget_link) => {
                // create the first blank proof tab
                menuwidget_link.send_message(MenuWidgetMsg::FileNew);
                self.menuwidget_link = Some(menuwidget_link);
                false
            },
            AppMsg::CreateTab { name, content } => {
                if let Some(tabcontainer_link) = &self.tabcontainer_link {
                    tabcontainer_link.send_message(TabbedContainerMsg::CreateTab { name, content });
                }
                true
            },
            AppMsg::RegisterProofName { name, link } => {
                self.proofs.insert(name, link);
                false
            }
            AppMsg::GetProofFromCurrentTab(f) => {
                if let Some(tabcontainer_link) = &self.tabcontainer_link {
                    let proofs = self.proofs.clone();
                    tabcontainer_link.send_message(TabbedContainerMsg::GetCurrentTab(Box::new(move |_, name| {
                        if let Some(link) = proofs.get(&*name) {
                            link.send_message(ProofWidgetMsg::CallOnProof(Box::new(move |prf| f(name, prf))));
                        }
                    })));
                }
                false
            }
        }
    }

    fn view(&self) -> Html {
        let exprwidget = html! {
            <div>
                <p>{ "Enter Expression:" }</p>
                <ExprEntry initial_contents="forall A, ((exists B, A -> B) & C & f(x, y | z)) <-> Q <-> R" onchange=self.link.callback(|(x, y)| AppMsg::ExprChanged(x, y)) />
            </div>
        };
        let resolution_fname: String = "resolution_example.bram".into();
        let resolution_fname_ = resolution_fname.clone();
        let tabview = html! {
            <TabbedContainer tab_ids=vec![resolution_fname.clone(), "Parser demo".into()] oncreate=self.link.callback(|link| AppMsg::TabbedContainerInit(link))>
                <ProofWidget verbose=true data=Some(include_bytes!("../../resolution_example.bram").to_vec()) oncreate=self.link.callback(move |link| AppMsg::RegisterProofName { name: resolution_fname_.clone(), link }) />
                { exprwidget }
            </TabbedContainer>
        };
        html! {
            <div>
                <MenuWidget parent=self.link.clone() oncreate=self.link.callback(|link| AppMsg::MenuWidgetInit(link)) />
                { tabview }
            </div>
        }
    }
}